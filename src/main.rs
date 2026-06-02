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
    KVM_EXIT_IO, KVM_EXIT_HLT, KVM_EXIT_FAIL_ENTRY, KVM_EXIT_INTERNAL_ERROR, kvm_regs as KvmRegs, kvm_run as KvmRun, kvm_sregs as KvmSregs,
    kvm_userspace_memory_region as KvmUserspaceMemoryRegion,
};

const KVM_VERSION: i32 = 12;
const GUEST_MEM_START: u64 = 0x1000;
const GUEST_MEM_SIZE: u64 = 2 * 4096;
const KVMIO: c_uint = 0xAE;
const CODE: [u8; 1] = [0xF4]; // op HLT 

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

    unsafe {
        std::ptr::copy_nonoverlapping(CODE.as_ptr(), vm_memory_mmap.ptr as *mut u8, CODE.len());
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

    k_sregs.cs.base = 0;
    k_sregs.cs.selector = 0;

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
            KVM_EXIT_HLT => break,
            KVM_EXIT_IO => handle_io_exit()?,
            KVM_EXIT_FAIL_ENTRY => return Err("failed entry".into()),
            KVM_EXIT_INTERNAL_ERROR => return Err("internal error".into()),
            other => {
                eprintln!("EXIT: {:?}", other);
                return Err("unhandled KVM exit: {other}".into());
            }
        }
    }

    Ok(())
}

fn handle_io_exit() -> Result<(), Box<dyn Error>> {
    Ok(())
}
