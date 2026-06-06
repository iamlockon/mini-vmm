use std::error::Error;

mod devices;
mod linux_loader;
mod memory;
mod vmm;

use vmm::Vmm;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--linux-info") => {
            let path = args.next().ok_or("missing bzImage path")?;
            let image = std::fs::read(path)?;
            let kernel = linux_loader::LinuxKernel::parse(image)?;
            kernel.dump_into();
            Ok(())
        }
        Some("--linux-layout") => {
            let path = args.next().ok_or("missing bzImage path")?;
            let image = std::fs::read(path)?;
            let kernel = linux_loader::LinuxKernel::parse(image)?;
            kernel.dump_into();

            let mut vmm = Vmm::empty()?;
            linux_loader::LinuxKernel::load(vmm.memory(), &kernel)?;

            vmm.setup_linux_protected_mode()?;
            vmm.setup_linux_regs(
                linux_loader::KERNEL_LOAD_ADDR,
                linux_loader::BOOT_PARAMS_ADDR,
            )?;
            Ok(())
        }
        _ => {
            let guest_path = std::env::args()
                .nth(1)
                .unwrap_or_else(|| "guest.bin".to_string());
            let guest_code = std::fs::read(guest_path)?;
            let mut vmm = Vmm::new(&guest_code)?;
            vmm.run()
        }
    }
}
