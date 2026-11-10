//! GPU sampling from the `IOKit` registry.
//!
//! Every GPU driver publishes an `IOAccelerator` service whose
//! `PerformanceStatistics` dictionary carries utilization and memory
//! counters, readable without special privileges. Apple Silicon reports
//! device, renderer and tiler utilization; Intel and AMD drivers report a
//! subset under slightly different keys, so missing counters read as zero.

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_void};

/// One GPU at one instant. Memory figures are in bytes.
#[derive(Debug, Clone, Default)]
pub struct GpuInfo {
    pub model: String,
    /// GPU cores, 0 when the driver does not say.
    pub cores: u32,
    /// Overall busy time in percent, 0..=100.
    pub device: f32,
    /// Fragment (renderer) busy time in percent.
    pub renderer: f32,
    /// Vertex (tiler) busy time in percent.
    pub tiler: f32,
    /// Memory the GPU is actively using.
    pub mem_in_use: u64,
    /// Memory allocated to the GPU, in use or not.
    pub mem_alloc: u64,
    /// Times the driver recovered from a GPU hang.
    pub recoveries: u64,
}

/// Samples every GPU in the registry, in registry order.
pub fn sample() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();
    // SAFETY: the matching dictionary is consumed by
    // IOServiceGetMatchingServices; every registry entry and the iterator are
    // released, and every CF object we create is released below.
    unsafe {
        let matching = IOServiceMatching(c"IOAccelerator".as_ptr());
        if matching.is_null() {
            return gpus;
        }
        let mut iter: u32 = 0;
        if IOServiceGetMatchingServices(0, matching, &raw mut iter) != 0 {
            return gpus;
        }
        loop {
            let entry = IOIteratorNext(iter);
            if entry == 0 {
                break;
            }
            if let Some(gpu) = read_entry(entry) {
                gpus.push(gpu);
            }
            IOObjectRelease(entry);
        }
        IOObjectRelease(iter);
    }
    gpus
}

/// Returns cumulative GPU time in nanoseconds for every process with a GPU
/// user client. Multiple command queues belonging to one process are summed.
pub fn process_times() -> HashMap<u32, u64> {
    let mut times = HashMap::new();
    // SAFETY: the matching dictionary is consumed by
    // IOServiceGetMatchingServices; every registry entry and iterator is
    // released, and CF properties are owned by `Property`.
    unsafe {
        let matching = IOServiceMatching(c"IOAccelerator".as_ptr());
        if matching.is_null() {
            return times;
        }
        let mut accelerators = 0;
        if IOServiceGetMatchingServices(0, matching, &raw mut accelerators) != 0 {
            return times;
        }
        loop {
            let accelerator = IOIteratorNext(accelerators);
            if accelerator == 0 {
                break;
            }
            let mut clients = 0;
            if IORegistryEntryGetChildIterator(accelerator, c"IOService".as_ptr(), &raw mut clients)
                == 0
            {
                loop {
                    let client = IOIteratorNext(clients);
                    if client == 0 {
                        break;
                    }
                    if let Some((pid, time)) = read_client(client) {
                        let total = times.entry(pid).or_insert(0);
                        *total = total.saturating_add(time);
                    }
                    IOObjectRelease(client);
                }
                IOObjectRelease(clients);
            }
            IOObjectRelease(accelerator);
        }
        IOObjectRelease(accelerators);
    }
    times
}

/// Reads the creator PID and cumulative GPU time from one GPU user client.
unsafe fn read_client(client: u32) -> Option<(u32, u64)> {
    // SAFETY: client is a live registry entry owned by the caller. Array
    // elements and dictionary values are borrowed while their owner lives.
    unsafe {
        let creator = Property::new(client, c"IOUserClientCreator")?;
        let creator = cf_string(creator.0)?;
        let pid = creator
            .strip_prefix("pid ")?
            .split([',', ' '])
            .next()?
            .parse()
            .ok()?;
        let usage = Property::new(client, c"AppUsage")?;
        if CFGetTypeID(usage.0) != CFArrayGetTypeID() {
            return Some((pid, 0));
        }
        let mut total = 0u64;
        for ix in 0..CFArrayGetCount(usage.0) {
            let entry = CFArrayGetValueAtIndex(usage.0, ix);
            if entry.is_null() || CFGetTypeID(entry) != CFDictionaryGetTypeID() {
                continue;
            }
            total = total.saturating_add(dict_number(entry, c"accumulatedGPUTime").unwrap_or(0));
        }
        Some((pid, total))
    }
}

/// Reads one `IOAccelerator` entry, `None` when it has no statistics.
unsafe fn read_entry(entry: u32) -> Option<GpuInfo> {
    // SAFETY: entry is a live registry entry owned by the caller.
    unsafe {
        let stats = Property::new(entry, c"PerformanceStatistics")?;
        if CFGetTypeID(stats.0) != CFDictionaryGetTypeID() {
            return None;
        }
        let num = |keys: &[&CStr]| {
            keys.iter()
                .find_map(|k| dict_number(stats.0, k))
                .unwrap_or(0)
        };
        let device = num(&[c"Device Utilization %", c"GPU Activity(%)"]);
        let model = Property::new(entry, c"model")
            .and_then(|p| cf_string(p.0))
            .unwrap_or_else(|| "GPU".to_owned());
        let cores = Property::new(entry, c"gpu-core-count")
            .and_then(|p| cf_number(p.0))
            .unwrap_or(0);
        Some(GpuInfo {
            model,
            cores: cores as u32,
            device: device.min(100) as f32,
            renderer: num(&[c"Renderer Utilization %"]).min(100) as f32,
            tiler: num(&[c"Tiler Utilization %"]).min(100) as f32,
            mem_in_use: num(&[c"In use system memory"]),
            mem_alloc: num(&[c"Alloc system memory"]),
            recoveries: num(&[c"recoveryCount"]),
        })
    }
}

/// An owned CF object copied out of the registry, released on drop.
struct Property(CFTypeRef);

impl Property {
    unsafe fn new(entry: u32, key: &CStr) -> Option<Self> {
        // SAFETY: the key string lives until it is released below.
        unsafe {
            let key = cf_str(key)?;
            let value = IORegistryEntryCreateCFProperty(entry, key, std::ptr::null(), 0);
            CFRelease(key);
            (!value.is_null()).then_some(Self(value))
        }
    }
}

impl Drop for Property {
    fn drop(&mut self) {
        // SAFETY: the property was returned by a Create function.
        unsafe { CFRelease(self.0) };
    }
}

/// Creates an owned `CFString`, `None` when CoreFoundation fails to allocate it.
/// The caller must release the result; releasing NULL aborts the process.
unsafe fn cf_str(s: &CStr) -> Option<CFTypeRef> {
    // SAFETY: s is a valid NUL-terminated UTF-8 string.
    let key = unsafe {
        CFStringCreateWithCString(std::ptr::null(), s.as_ptr(), K_CF_STRING_ENCODING_UTF8)
    };
    (!key.is_null()).then_some(key)
}

unsafe fn dict_number(dict: CFTypeRef, key: &CStr) -> Option<u64> {
    // SAFETY: dict is a CFDictionary; the value is borrowed, not owned.
    unsafe {
        let key = cf_str(key)?;
        let value = CFDictionaryGetValue(dict, key);
        CFRelease(key);
        if value.is_null() {
            None
        } else {
            cf_number(value)
        }
    }
}

unsafe fn cf_number(value: CFTypeRef) -> Option<u64> {
    // SAFETY: value is a live CF object; it is type checked before reading.
    unsafe {
        if CFGetTypeID(value) != CFNumberGetTypeID() {
            return None;
        }
        let mut n: i64 = 0;
        CFNumberGetValue(value, K_CF_NUMBER_SINT64_TYPE, (&raw mut n).cast())
            .then(|| n.max(0) as u64)
    }
}

unsafe fn cf_string(value: CFTypeRef) -> Option<String> {
    // SAFETY: value is a live CF object; it is type checked before reading.
    unsafe {
        if CFGetTypeID(value) != CFStringGetTypeID() {
            return None;
        }
        let mut buf = [0 as c_char; 256];
        CFStringGetCString(
            value,
            buf.as_mut_ptr(),
            buf.len() as isize,
            K_CF_STRING_ENCODING_UTF8,
        )
        .then(|| CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
    }
}

type CFTypeRef = *const c_void;
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const K_CF_NUMBER_SINT64_TYPE: isize = 4;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayGetCount(array: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, ix: isize) -> CFTypeRef;
    fn CFNumberGetTypeID() -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFNumberGetValue(number: CFTypeRef, kind: isize, out: *mut c_void) -> bool;
    fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const c_char, enc: u32) -> CFTypeRef;
    fn CFStringGetCString(s: CFTypeRef, buf: *mut c_char, len: isize, enc: u32) -> bool;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CFTypeRef;
    fn IOServiceGetMatchingServices(port: u32, matching: CFTypeRef, iter: *mut u32) -> i32;
    fn IOIteratorNext(iter: u32) -> u32;
    fn IOObjectRelease(object: u32) -> i32;
    fn IORegistryEntryGetChildIterator(entry: u32, plane: *const c_char, iter: *mut u32) -> i32;
    fn IORegistryEntryCreateCFProperty(
        entry: u32,
        key: CFTypeRef,
        alloc: CFTypeRef,
        options: u32,
    ) -> CFTypeRef;
}

#[cfg(test)]
mod tests {
    #[test]
    fn samples_the_gpu() {
        for gpu in super::sample() {
            eprintln!("{gpu:?}");
            assert!(gpu.device <= 100.0);
        }
    }
}
