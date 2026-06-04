use std::error::Error;

use kvm_bindings::{KVM_EXIT_IO_IN, KVM_EXIT_IO_OUT};

pub type KvmRunIo = kvm_bindings::kvm_run__bindgen_ty_1__bindgen_ty_4;

/// Handle IO for different port devices.
pub trait PortDevice {
    fn handles(&self, port: u16) -> bool;
    fn io_out(&mut self, port: u16, data: &[u8]) -> Result<(), Box<dyn Error>>;
    fn io_in(&mut self, port: u16, data: &mut [u8]) -> Result<(), Box<dyn Error>>;
}

pub struct DebugPort;

impl PortDevice for DebugPort {
    fn handles(&self, port: u16) -> bool {
        port == 0xe9
    }

    fn io_out(&mut self, _port: u16, data: &[u8]) -> Result<(), Box<dyn Error>> {
        for byte in data {
            print!("{}", *byte as char);
        }
        Ok(())
    }

    fn io_in(&mut self, _port: u16, data: &mut [u8]) -> Result<(), Box<dyn Error>> {
        // Return a deterministic placeholder for reads from the debug port.
        data.fill(0);
        Ok(())
    }
}

#[derive(Default)]
pub struct SerialPort {
    interrupt_enable: u8,
    fifo_control: u8,
    line_control: u8,
    modem_control: u8,
}

impl SerialPort {
    pub fn new() -> Self {
        Default::default()
    }
}

impl PortDevice for SerialPort {
    fn handles(&self, port: u16) -> bool {
        // COM1 occupies the standard PC I/O port range 0x3f8..=0x3fd.
        matches!(port, 0x3f8..=0x3fd)
    }

    fn io_out(&mut self, port: u16, data: &[u8]) -> Result<(), Box<dyn Error>> {
        match port {
            0x3f8 => {
                // Transmit holding register: bytes written here are serial output.
                for byte in data {
                    print!("{}", *byte as char);
                }
            }
            0x3f9 => {
                // Interrupt enable register.
                if let Some(byte) = data.last() {
                    self.interrupt_enable = *byte;
                }
            }
            0x3fa => {
                // FIFO control register on write.
                if let Some(byte) = data.last() {
                    self.fifo_control = *byte;
                }
            }
            0x3fb => {
                // Line control register.
                if let Some(byte) = data.last() {
                    self.line_control = *byte;
                }
            }
            0x3fc => {
                // Modem control register.
                if let Some(byte) = data.last() {
                    self.modem_control = *byte;
                }
            }
            0x3fd => {
                // Line status register is read-only for this minimal model.
            }
            other => {
                eprintln!("unhandled serial out: port={other:#x} data={data:x?}");
            }
        }
        Ok(())
    }

    fn io_in(&mut self, port: u16, data: &mut [u8]) -> Result<(), Box<dyn Error>> {
        match port {
            // Receive buffer: no input available.
            0x3f8 => data.fill(0),
            0x3f9 => data.fill(self.interrupt_enable),
            // Interrupt identification register: bit 0 set means no interrupt pending.
            0x3fa => data.fill(0x01),
            0x3fb => data.fill(self.line_control),
            0x3fc => data.fill(self.modem_control),
            // Line status register:
            // 0x20 = transmit holding register empty
            // 0x40 = transmitter empty
            0x3fd => data.fill(0x20 | 0x40),
            other => {
                eprintln!("unhandled io in port: {other}");
                data.fill(0);
            }
        }
        Ok(())
    }
}

pub fn handle_io_exit(
    io: KvmRunIo,
    kvm_run_mmap_ptr: *mut std::os::raw::c_void,
    devices: &mut [&mut dyn PortDevice],
) -> Result<(), Box<dyn Error>> {
    let run_base = kvm_run_mmap_ptr.cast::<u8>();
    let data_len = io.size as usize * io.count as usize;

    match io.direction as u32 {
        KVM_EXIT_IO_OUT => {
            // For OUT exits, KVM has placed the guest-provided bytes in the
            // kvm_run data area at data_offset.
            let data = unsafe {
                std::slice::from_raw_parts(run_base.add(io.data_offset as usize), data_len)
            };
            if let Some(device) = devices.iter_mut().find(|d| d.handles(io.port)) {
                device.io_out(io.port, data)?;
            } else {
                eprintln!("unhandled io out: port={:#x} data={:x?}", io.port, data);
            }
        }
        KVM_EXIT_IO_IN => {
            // For IN exits, userspace must write response bytes into this
            // shared buffer before the next KVM_RUN.
            let data = unsafe {
                std::slice::from_raw_parts_mut(run_base.add(io.data_offset as usize), data_len)
            };

            if let Some(device) = devices.iter_mut().find(|d| d.handles(io.port)) {
                device.io_in(io.port, data)?;
            } else {
                data.fill(0);
                eprintln!("unhandled io in: port={:#x}", io.port);
            }
        }
        other => {
            return Err(format!("unknown io direction: {other}").into());
        }
    }
    Ok(())
}
