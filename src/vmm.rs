use std::{
    error::Error,
    fs::{File, OpenOptions},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::OpenOptionsExt,
    },
};

use std::os::raw::{c_uint, c_ulong};

use kvm_bindings::{
    KVM_EXIT_FAIL_ENTRY, KVM_EXIT_HLT, KVM_EXIT_INTERNAL_ERROR, KVM_EXIT_IO, KVM_EXIT_MMIO,
    kvm_regs as KvmRegs, kvm_run as KvmRun, kvm_sregs as KvmSregs,
    kvm_userspace_memory_region as KvmUserspaceMemoryRegion,
};

use crate::{
    devices::{DebugPort, PortDevice, SerialPort, handle_io_exit},
    memory::{GuestMemory, Mmap},
};

const KVM_VERSION: i32 = 12;
const GUEST_ENTRY: u64 = 0x1000;
const GUEST_MEM_START: u64 = 0;
const GUEST_MEM_SIZE: u64 = 64 * 1024 * 1024; // 64 MiB.
const KVMIO: c_uint = 0xAE;

const KVM_GET_API_VERSION: c_ulong = libc::_IO(KVMIO, 0x00);
const KVM_CREATE_VM: c_ulong = libc::_IO(KVMIO, 0x01);
const KVM_CREATE_VCPU: c_ulong = libc::_IO(KVMIO, 0x41);
const KVM_SET_USER_MEMORY_REGION: c_ulong = libc::_IOW::<KvmUserspaceMemoryRegion>(KVMIO, 0x46);
const KVM_SET_REGS: c_ulong = libc::_IOW::<KvmRegs>(KVMIO, 0x82);
const KVM_GET_SREGS: c_ulong = libc::_IOR::<KvmSregs>(KVMIO, 0x83);
const KVM_SET_SREGS: c_ulong = libc::_IOW::<KvmSregs>(KVMIO, 0x84);
const KVM_GET_VCPU_MMAP_SIZE: c_ulong = libc::_IO(KVMIO, 0x04);
const KVM_RUN: c_ulong = libc::_IO(KVMIO, 0x80);

pub struct Vmm {
    // Keep these file descriptors alive for the lifetime of the VM/VCPU.
    _kvm_file: File,
    _vm_fd: OwnedFd,
    vcpu_fd: OwnedFd,
    // Keep guest RAM mapped while the VM can access it.
    guest_memory: GuestMemory,
    kvm_run_mmap: Mmap,
}

impl Vmm {
    pub fn new(guest_code: &[u8]) -> Result<Self, Box<dyn Error>> {
        let mut vmm = Self::empty()?;
        vmm.guest_memory.load(GUEST_ENTRY, guest_code)?;
        Ok(vmm)
    }

    pub fn empty() -> Result<Self, Box<dyn Error>> {
        let kvm_file = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_RDWR | libc::O_CLOEXEC)
            .open("/dev/kvm")?;

        let kvm_fd = kvm_file.as_raw_fd();
        check_kvm_version(kvm_fd)?;

        let vm_fd = create_vm(kvm_fd)?;
        let vcpu_fd = create_vcpu(vm_fd.as_raw_fd())?;

        println!(
            "kvm fd {kvm_fd}, vm fd: {}, vcpu fd: {}",
            vm_fd.as_raw_fd(),
            vcpu_fd.as_raw_fd()
        );

        let guest_memory = GuestMemory::new(GUEST_MEM_SIZE)?;
        register_guest_memory(vm_fd.as_raw_fd(), &guest_memory)?;
        setup_regs(vcpu_fd.as_raw_fd())?;
        setup_real_mode_sregs(vcpu_fd.as_raw_fd())?;
        let kvm_run_mmap = mmap_kvm_run(kvm_fd, vcpu_fd.as_raw_fd())?;

        Ok(Self {
            _kvm_file: kvm_file,
            _vm_fd: vm_fd,
            vcpu_fd,
            guest_memory: guest_memory,
            kvm_run_mmap,
        })
    }

    pub fn run(&mut self) -> Result<(), Box<dyn Error>> {
        let mut debug_port = DebugPort;
        let mut serial_port = SerialPort::new();
        let mut devices: [&mut dyn PortDevice; 2] = [&mut debug_port, &mut serial_port];

        loop {
            let ret = unsafe { libc::ioctl(self.vcpu_fd.as_raw_fd(), KVM_RUN, 0) };
            if ret != 0 {
                eprintln!("KVM+RUN errored");
            }

            let k_run: &KvmRun = unsafe { &*(self.kvm_run_mmap.ptr as *const KvmRun) };

            match k_run.exit_reason {
                KVM_EXIT_HLT => {
                    println!();
                    break;
                }
                KVM_EXIT_IO => {
                    handle_io_exit(
                        unsafe { k_run.__bindgen_anon_1.io },
                        self.kvm_run_mmap.ptr,
                        &mut devices,
                    )?;
                }
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

    pub fn memory(&mut self) -> &mut GuestMemory {
        &mut self.guest_memory
    }
}

fn check_kvm_version(kvm_fd: i32) -> Result<(), Box<dyn Error>> {
    let kvm_version = unsafe { libc::ioctl(kvm_fd, KVM_GET_API_VERSION, 0) };
    println!("kvm version {kvm_version}");

    if kvm_version < 0 {
        eprintln!("error getting kvm version");
        return Err(std::io::Error::last_os_error().into());
    }

    if kvm_version != KVM_VERSION {
        eprintln!("current kvm version: {kvm_version}, required kvm version: {KVM_VERSION}");
        return Err("kvm version not supported".into());
    }

    Ok(())
}

fn create_vm(kvm_fd: i32) -> Result<OwnedFd, Box<dyn Error>> {
    let vm_fd = unsafe { libc::ioctl(kvm_fd, KVM_CREATE_VM, 0) };
    if vm_fd < 0 {
        eprintln!("vm creation error");
        return Err(std::io::Error::last_os_error().into());
    }

    Ok(unsafe { OwnedFd::from_raw_fd(vm_fd) })
}

fn create_vcpu(vm_fd: i32) -> Result<OwnedFd, Box<dyn Error>> {
    let vcpu_fd = unsafe { libc::ioctl(vm_fd, KVM_CREATE_VCPU, 0) };
    if vcpu_fd < 0 {
        eprintln!("vcpu creation error");
        return Err(std::io::Error::last_os_error().into());
    }

    Ok(unsafe { OwnedFd::from_raw_fd(vcpu_fd) })
}

fn register_guest_memory(vm_fd: i32, guest_memory: &GuestMemory) -> Result<(), Box<dyn Error>> {
    let userspace_memory_region = KvmUserspaceMemoryRegion {
        slot: 0,
        flags: 0,
        // Map RAM from guest physical 0 so guest addresses equal mmap offsets.
        guest_phys_addr: GUEST_MEM_START,
        memory_size: GUEST_MEM_SIZE,
        // Host userspace address backing the guest RAM.
        userspace_addr: guest_memory.userspace_addr(),
    };

    let ret = unsafe { libc::ioctl(vm_fd, KVM_SET_USER_MEMORY_REGION, &userspace_memory_region) };
    if ret != 0 {
        eprintln!("error setting user memory region");
        return Err(std::io::Error::last_os_error().into());
    }

    Ok(())
}

fn setup_regs(vcpu_fd: i32) -> Result<(), Box<dyn Error>> {
    let regs = KvmRegs {
        // Start executing the guest image loaded at 0x1000.
        rip: GUEST_ENTRY,
        // Real-mode stack pointer. With SS.base=0, stack accesses use
        // physical addresses below 0x2000 as the stack grows downward.
        rsp: 0x2000,
        // Bit 1 is reserved and should always be set.
        rflags: 0x2,
        ..Default::default()
    };

    let ret = unsafe { libc::ioctl(vcpu_fd, KVM_SET_REGS, &regs) };
    if ret != 0 {
        eprintln!("error setting kvm_regs");
        return Err(std::io::Error::last_os_error().into());
    }

    Ok(())
}

fn setup_real_mode_sregs(vcpu_fd: i32) -> Result<(), Box<dyn Error>> {
    let mut sregs = KvmSregs::default();
    let ret = unsafe { libc::ioctl(vcpu_fd, KVM_GET_SREGS, &sregs) };
    if ret != 0 {
        eprintln!("error getting kvm_sregs");
        return Err(std::io::Error::last_os_error().into());
    }

    // Real mode prep: clear CR0.PE, the protection-enable bit.
    sregs.cr0 &= !1;

    // Code segment used for instruction fetches.
    sregs.cs.base = 0;
    sregs.cs.selector = 0;
    sregs.cs.limit = 0xffff;

    // Data segment used for most memory reads/writes.
    sregs.ds.base = 0;
    sregs.ds.selector = 0;
    sregs.ds.limit = 0xffff;

    // Extra segments used by some string/legacy instructions.
    sregs.es.base = 0;
    sregs.es.selector = 0;
    sregs.es.limit = 0xffff;

    sregs.fs.base = 0;
    sregs.fs.selector = 0;
    sregs.fs.limit = 0xffff;

    sregs.gs.base = 0;
    sregs.gs.selector = 0;
    sregs.gs.limit = 0xffff;

    // Stack segment used with SP/RSP for push, pop, call, and ret.
    sregs.ss.base = 0;
    sregs.ss.selector = 0;
    sregs.ss.limit = 0xffff;

    let ret = unsafe { libc::ioctl(vcpu_fd, KVM_SET_SREGS, &sregs) };
    if ret != 0 {
        eprintln!("error setting kvm_sregs");
        return Err(std::io::Error::last_os_error().into());
    }

    Ok(())
}

fn mmap_kvm_run(kvm_fd: i32, vcpu_fd: i32) -> Result<Mmap, Box<dyn Error>> {
    let vcpu_mmap_size = unsafe { libc::ioctl(kvm_fd, KVM_GET_VCPU_MMAP_SIZE, 0) };
    if vcpu_mmap_size < 0 {
        eprintln!("error getting vcpu mmap size: {vcpu_mmap_size}");
        return Err(std::io::Error::last_os_error().into());
    }

    println!("vcpu mmap size: {vcpu_mmap_size} bytes");
    Mmap::shared(vcpu_fd, vcpu_mmap_size as usize)
}
