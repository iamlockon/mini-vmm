/// For loading linux image
use std::error::Error;

pub struct LinuxKernel {
    /// Complete bzImage file contents.
    image: Vec<u8>,
    /// Number of 512-byte setup sectors after the boot sector.
    pub setup_sects: u8,
    /// Total size of the real-mode setup area, including the first boot sector.
    pub setup_size: usize,
    /// Linux x86 boot protocol version from the setup header.
    pub protocol_version: u16,
    /// Boot-protocol flags that describe loader/kernel requirements.
    pub loadflags: u8,
    /// Default protected-mode entry address for the 32-bit kernel code.
    pub code32_start: u32,
    /// Maximum supported kernel command-line length.
    pub cmdline_size: u32,
    /// Highest allowed physical address for an initrd image.
    pub initrd_addr_max: u32,
    /// Required alignment for loading the protected-mode kernel payload.
    pub kernel_alignment: u32,
}

impl LinuxKernel {
    pub fn parse(image: Vec<u8>) -> Result<Self, Box<dyn Error>> {
        Self::require_len(&image, 0x238 + 4)?;

        let boot_flag = Self::read_u16(&image, 0x1fe)?;
        if boot_flag != 0xaa55 {
            return Err(format!("invalid boot flag: {boot_flag:#x}").into());
        }

        if &image[0x202..0x206] != b"HdrS" {
            return Err("missing Linux boot protocol header: HdrS".into());
        }

        let raw_setup_sects = image[0x1f1];
        let setup_sects = if raw_setup_sects == 0 {
            4
        } else {
            raw_setup_sects
        };
        let setup_size = (setup_sects as usize + 1) * 512;

        if image.len() <= setup_size {
            return Err(format!(
                "kernel image too small: image={} setup_size={}",
                image.len(),
                setup_size,
            )
            .into());
        }

        Ok(Self {
            protocol_version: Self::read_u16(&image, 0x206)?,
            loadflags: image[0x211],
            code32_start: Self::read_u32(&image, 0x214)?,
            initrd_addr_max: Self::read_u32(&image, 0x22c)?,
            kernel_alignment: Self::read_u32(&image, 0x230)?,
            cmdline_size: Self::read_u32(&image, 0x238)?,
            image,
            setup_sects,
            setup_size,
        })
    }

    pub fn setup(&self) -> &[u8] {
        &self.image[..self.setup_size]
    }

    pub fn protected_mode_kernel(&self) -> &[u8] {
        &self.image[self.setup_size..]
    }

    pub fn dump_into(&self) {
        println!("Linux bzImage:");
        println!("  setup_sects: {}", self.setup_sects);
        println!("  setup_size: {} bytes", self.setup_size);
        println!("  protocol_version: {:#x}", self.protocol_version);
        println!("  loadflags: {:#x}", self.loadflags);
        println!("  code32_start: {:#x}", self.code32_start);
        println!("  cmdline_size: {}", self.cmdline_size);
        println!("  initrd_addr_max: {:#x}", self.initrd_addr_max);
        println!("  kernel_alignment: {:#x}", self.kernel_alignment);
        println!(
            "  protected kernel size: {} bytes",
            self.protected_mode_kernel().len()
        );
    }

    fn require_len(image: &[u8], len: usize) -> Result<(), Box<dyn Error>> {
        if image.len() < len {
            return Err(format!("kernel image too small: {} < {}", image.len(), len).into());
        }
        Ok(())
    }

    fn read_u16(image: &[u8], offset: usize) -> Result<u16, Box<dyn Error>> {
        Self::require_len(image, offset + 2)?;
        Ok(u16::from_le_bytes([image[offset], image[offset + 1]]))
    }

    fn read_u32(image: &[u8], offset: usize) -> Result<u32, Box<dyn Error>> {
        Self::require_len(image, offset + 4)?;
        Ok(u32::from_le_bytes([
            image[offset],
            image[offset + 1],
            image[offset + 2],
            image[offset + 3],
        ]))
    }
}
