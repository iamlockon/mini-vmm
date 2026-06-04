use std::error::Error;

mod devices;
mod memory;
mod vmm;

use vmm::Vmm;

fn main() -> Result<(), Box<dyn Error>> {
    let guest_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "guest.bin".to_string());

    let guest_code = std::fs::read(&guest_path)?;
    let mut vmm = Vmm::new(&guest_code)?;
    vmm.run()
}
