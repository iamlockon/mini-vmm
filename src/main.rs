use std::{
    error::Error,
    fs::OpenOptions,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::OpenOptionsExt,
    },
};

use std::os::raw::{c_uint, c_ulong};

use kvm_bindings::{
    KVM_EXIT_FAIL_ENTRY, KVM_EXIT_HLT, KVM_EXIT_INTERNAL_ERROR, KVM_EXIT_IO, KVM_EXIT_IO_IN, KVM_EXIT_IO_OUT, KVM_EXIT_MMIO, kvm_regs as KvmRegs, kvm_run as KvmRun, kvm_sregs as KvmSregs, kvm_userspace_memory_region as KvmUserspaceMemoryRegion
};

const KVM_VERSION: i32 = 12;
const GUEST_MEM_START: u64 = 0x1000;
const GUEST_MEM_SIZE: u64 = 2 * 4096;
const KVMIO: c_uint = 0xAE;

// operations
const KVM_GET_API_VERSION: c_ulong = libc::_IO(KVMIO, 0x00);
const KVM_CREATE_VM: c_ulong = libc::_IO(KVMIO, 0x01);
const KVM_CREATE_VCPU: c_ulong = libc::_IO(KVMIO, 0x41);
const KVM_SET_USER_MEMORY_REGION: c_ulong = libc::_IOW::<KvmUserspaceMemoryRegion>(KVMIO, 0x46);
const KVM_SET_REGS: c_ulong = libc::_IOW::<KvmRegs>(KVMIO, 0x82);
const KVM_GET_SREGS: c_ulong = libc::_IOR::<KvmSregs>(KVMIO, 0x83);
const KVM_SET_SREGS: c_ulong = libc::_IOW::<KvmSregs>(KVMIO, 0x84);
const KVM_GET_VCPU_MMAP_SIZE: c_ulong = libc::_IO(KVMIO, 0x04);
const KVM_RUN: c_ulong = libc::_IO(KVMIO, 0x80);

type KvmRunIo = kvm_bindings::kvm_run__bindgen_ty_1__bindgen_ty_4;

struct Mmap {
    ptr: *mut std::os::raw::c_void,
    len: usize,
}

// impls
impl Drop for Mmap {
    fn drop(&mut self) {
        println!("calling libc:munmap");
        unsafe { libc::munmap(self.ptr, self.len) };
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let file = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_RDWR | libc::O_CLOEXEC)
        .open("/dev/kvm")?;

    let kvm_fd = file.as_raw_fd();

    let kvm_version = unsafe { libc::ioctl(kvm_fd, KVM_GET_API_VERSION, 0) };

    println!("kvm version {kvm_version}");

    if kvm_version < 0 {
        let last_os_error = std::io::Error::last_os_error();
        eprintln!("error getting kvm version");
        return Err(last_os_error.into());
    };

    if kvm_version != KVM_VERSION {
        eprintln!("current kvm version: {kvm_version}, required kvm version: {KVM_VERSION}");
        return Err("kvm version not supported".into());
    }

    let vm_fd = unsafe { libc::ioctl(kvm_fd, KVM_CREATE_VM, 0) };

    if vm_fd < 0 {
        let last_os_error = std::io::Error::last_os_error();
        eprintln!("vm creation error");
        return Err(last_os_error.into());
    }

    let vm_fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(vm_fd) };

    let vcpu_fd = unsafe { libc::ioctl(vm_fd.as_raw_fd(), KVM_CREATE_VCPU, 0) };

    if vcpu_fd < 0 {
        let last_os_error = std::io::Error::last_os_error();
        eprintln!("vcpu creation error");
        return Err(last_os_error.into());
    }

    let vcpu_fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(vcpu_fd) };

    println!(
        "kvm fd {kvm_fd}, vm fd: {}, vcpu fd: {}",
        vm_fd.as_raw_fd(),
        vcpu_fd.as_raw_fd()
    );

    let vm_memory_mmap = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            GUEST_MEM_SIZE as usize,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_ANONYMOUS | libc::MAP_PRIVATE,
            -1,
            0,
        )
    };

    if vm_memory_mmap == libc::MAP_FAILED {
        let last_os_error = std::io::Error::last_os_error();
        eprintln!("vm memory map failed");
        return Err(last_os_error.into());
    }

    let vm_memory_mmap = Mmap {
        ptr: vm_memory_mmap,
        len: GUEST_MEM_SIZE as usize,
    };

    let guest_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "guest.bin".to_string());

    let guest_code = std::fs::read(&guest_path)?;
    if guest_code.len() > GUEST_MEM_SIZE as usize {
        return Err(format!(
            "guest image too large: {} bytes > {} bytes",
            guest_code.len(),
            GUEST_MEM_SIZE,
        ).into());
    }
    
    unsafe {
        std::ptr::copy_nonoverlapping(
            guest_code.as_ptr(),
            vm_memory_mmap.ptr as *mut u8,
            guest_code.len(),
        );
    }

    let vm_memory_addr = vm_memory_mmap.ptr as u64;

    let userspace_memory_region = KvmUserspaceMemoryRegion {
        slot: 0,
        flags: 0,
        guest_phys_addr: GUEST_MEM_START,
        memory_size: GUEST_MEM_SIZE,
        userspace_addr: vm_memory_addr,
    };

    let ret = unsafe {
        libc::ioctl(
            vm_fd.as_raw_fd(),
            KVM_SET_USER_MEMORY_REGION,
            &userspace_memory_region,
        )
    };

    if ret != 0 {
        let last_os_error = std::io::Error::last_os_error();
        eprintln!("error setting user memory region");
        return Err(last_os_error.into());
    }

    let k_regs = KvmRegs {
        rip: GUEST_MEM_START,
        rsp: 0x2000, // 2 * 4096
        rflags: 0x2,
        ..Default::default()
    };

    let ret = unsafe { libc::ioctl(vcpu_fd.as_raw_fd(), KVM_SET_REGS, &k_regs) };

    if ret != 0 {
        let last_os_error = std::io::Error::last_os_error();
        eprintln!("error setting kvm_regs");
        return Err(last_os_error.into());
    }

    let mut k_sregs = KvmSregs::default();
    let ret = unsafe { libc::ioctl(vcpu_fd.as_raw_fd(), KVM_GET_SREGS, &k_sregs) };

    if ret != 0 {
        let last_os_error = std::io::Error::last_os_error();
        eprintln!("error getting kvm_sregs");
        return Err(last_os_error.into());
    }

    // real mode prep 
    k_sregs.cr0 &= !1; // clear PE bit

    // Code segment
    k_sregs.cs.base = 0;
    k_sregs.cs.selector = 0;
    k_sregs.cs.limit = 0xffff;

    // Data segment
    k_sregs.ds.base = 0;
    k_sregs.ds.selector = 0;
    k_sregs.ds.limit = 0xffff;

    // Extra segment
    k_sregs.es.base = 0;
    k_sregs.es.selector = 0;
    k_sregs.es.limit = 0xffff;
    
    k_sregs.fs.base = 0;
    k_sregs.fs.selector = 0;
    k_sregs.fs.limit = 0xffff;
    
    k_sregs.gs.base = 0;
    k_sregs.gs.selector = 0;
    k_sregs.gs.limit = 0xffff;

    // Stack segment  
    k_sregs.ss.base = 0;
    k_sregs.ss.selector = 0;
    k_sregs.ss.limit = 0xffff;
    
    let ret = unsafe { libc::ioctl(vcpu_fd.as_raw_fd(), KVM_SET_SREGS, &k_sregs) };

    if ret != 0 {
        let last_os_error = std::io::Error::last_os_error();
        eprintln!("error setting kvm_sregs");
        return Err(last_os_error.into());
    }

    let vcpu_mmap_size = unsafe { libc::ioctl(kvm_fd, KVM_GET_VCPU_MMAP_SIZE, 0) };

    if vcpu_mmap_size < 0 {
        let last_os_error = std::io::Error::last_os_error();
        eprintln!("error getting vcpu mmap size: {vcpu_mmap_size}");
        return Err(last_os_error.into());
    }

    println!("vcpu mmap size: {vcpu_mmap_size} bytes");

    let kvm_run_mmap = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            vcpu_mmap_size as usize,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            vcpu_fd.as_raw_fd(),
            0,
        )
    };

    if kvm_run_mmap == libc::MAP_FAILED {
        let last_os_error = std::io::Error::last_os_error();
        eprintln!("kvm_run mmap failed");
        return Err(last_os_error.into());
    }

    let kvm_run_mmap = Mmap {
        ptr: kvm_run_mmap,
        len: vcpu_mmap_size as usize,
    };

    loop {
        let ret = unsafe { libc::ioctl(vcpu_fd.as_raw_fd(), KVM_RUN, 0) };

        if ret != 0 {
            eprintln!("KVM+RUN errored");
        }

        let k_run: &KvmRun = unsafe { &*(kvm_run_mmap.ptr as *const KvmRun) };

        match k_run.exit_reason {
            KVM_EXIT_HLT => {
                println!();
                break;
            },
            KVM_EXIT_IO => handle_io_exit(unsafe { k_run.__bindgen_anon_1.io }, kvm_run_mmap.ptr)?,
            KVM_EXIT_FAIL_ENTRY => return Err("failed entry".into()),
            KVM_EXIT_INTERNAL_ERROR => return Err("internal error".into()),
            KVM_EXIT_MMIO => return Err("MMIO exit: guest accessed unmapped memory".into()),
            other => {
                eprintln!("EXIT: {:?}", other);
                return Err(format!("unhandled KVM exit: {other}").into());
            }
        }
    }

    Ok(())
}

fn handle_io_exit(
    io: KvmRunIo,
    kvm_run_mmap_ptr: *mut std::os::raw::c_void,
) -> Result<(), Box<dyn Error>> {
    match io.direction as u32 {
        KVM_EXIT_IO_OUT => {
            let run_base = kvm_run_mmap_ptr.cast::<u8>();
            let data_len = io.size as usize * io.count as usize;

            let data = unsafe {
                std::slice::from_raw_parts(run_base.add(io.data_offset as usize), data_len)
            };

            match io.port {
                0xe9 => {
                    for byte in data {
                        print!("{}", *byte as char);
                    }
                }
                _ => {
                    eprintln!("unhandled io out: port:{:#x} data={:x?}", io.port, data);
                }
            }
        }
        KVM_EXIT_IO_IN => {
            let run_base = kvm_run_mmap_ptr.cast::<u8>();
            let data_len = io.size as usize * io.count as usize;

            let data = unsafe {
                std::slice::from_raw_parts_mut(run_base.add(io.data_offset as usize), data_len)
            };

            match io.port {
                _ => {
                    data.fill(0); // placeholder value for unhandled port
                    eprintln!("unhandled io in: port={:#x}", io.port);
                }
            }
        }
        other => {
            return Err(format!("unknown io direction: {other}").into());
        }
    }
    Ok(())
}
