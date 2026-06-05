use std::{error::Error, os::raw::c_void};

pub struct Mmap {
    pub ptr: *mut c_void,
    len: usize,
}

impl Mmap {
    pub fn anonymous(len: usize) -> Result<Self, Box<dyn Error>> {
        // Allocate host userspace memory that will back guest RAM.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_ANONYMOUS | libc::MAP_PRIVATE,
                -1,
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error().into());
        }

        Ok(Self { ptr, len })
    }

    pub fn shared(fd: i32, len: usize) -> Result<Self, Box<dyn Error>> {
        // Map the shared kvm_run structure exposed by the VCPU fd.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error().into());
        }

        Ok(Self { ptr, len })
    }
}

impl Drop for Mmap {
    fn drop(&mut self) {
        println!("calling libc:munmap");
        unsafe { libc::munmap(self.ptr, self.len) };
    }
}

pub struct GuestMemory {
    mmap: Mmap,
    size: u64,
}

impl GuestMemory {
    pub fn new(size: u64) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            mmap: Mmap::anonymous(size as usize)?,
            size,
        })
    }

    pub fn load(&mut self, guest_addr: u64, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
        self.write(guest_addr, bytes)
    }

    pub fn userspace_addr(&self) -> u64 {
        self.mmap.ptr as u64
    }

    /// Translate guest address to host address
    pub fn guest_to_host(&self, guest_addr: u64) -> Result<*mut u8, Box<dyn Error>> {
        if guest_addr >= self.size {
            return Err(format!(
                "guest address out of range: {guest_addr:#x} >= {:#x}",
                self.size,
            ).into());
        }

        Ok(unsafe { (self.mmap.ptr as *mut u8).add(guest_addr as usize) })
    }

    pub fn write(&mut self, guest_addr: u64, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
        let end = guest_addr as usize + bytes.len();
        if end > self.size as usize {
            return Err(format!(
                "guest write out of range: end={end:#x} size={:#x}",
                self.size
            ).into());
        }

        let host_addr = self.guest_to_host(guest_addr)?;
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), host_addr, bytes.len());
        }

        Ok(())
    }
}
