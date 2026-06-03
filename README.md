# mini-vmm

`mini-vmm` is a small Rust/KVM example that creates a virtual machine,
maps a tiny guest memory region, loads a raw guest binary into it, and runs
one virtual CPU until the guest halts.

The project is intentionally minimal. It uses Linux KVM ioctls directly
through `libc` and `kvm-bindings` instead of a higher-level VMM framework.
That makes it useful for seeing the basic control flow:

1. Open `/dev/kvm` and check the KVM API version.
2. Create a VM and one vCPU.
3. Allocate anonymous userspace memory for the guest.
4. Register that memory with KVM at guest physical address `0x1000`.
5. Copy a raw guest image into the mapped memory.
6. Set `rip` to `0x1000` and run the vCPU with `KVM_RUN`.
7. Handle a small set of exits: `hlt`, I/O, failed entry, and internal error.

The VMM currently treats output to I/O port `0xe9` as debug text and prints
the bytes to stdout. The sample guest writes `A` to that port and then halts.

## Requirements

- Linux with KVM support enabled
- Permission to open `/dev/kvm`
- Rust toolchain
- `nasm` for assembling the sample guest

## Building A Guest With NASM

The VMM expects a flat binary image, not an ELF executable. NASM can produce
that directly with the `bin` output format:

```bash
nasm -f bin guest.asm -o guest.bin
```

The included `guest.asm` is 16-bit code:

```asm
bits 16

mov dx, 0xe9
mov al, 'A'
out dx, al

hlt
```

Because the VMM loads the guest bytes at guest physical address `0x1000` and
starts execution with `rip = 0x1000`, the first byte of `guest.bin` is the
first instruction executed by the vCPU.

## Running

Build the sample guest, then run the VMM:

```bash
nasm -f bin guest.asm -o guest.bin
cargo run
```

You can also pass an explicit guest image path:

```bash
cargo run -- path/to/guest.bin
```

The guest image must fit in the configured guest memory region, which is
currently 8192 bytes.
