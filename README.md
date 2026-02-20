# Alpheratz

Canicula OS 的 UEFI Bootloader 应用，用于加载 Linux EFI Stub 和 Canicula OS。

支持 x86_64、AArch64、RISC-V 64、LoongArch64 四个架构，支持基于 UEFI 的网络驱动在启动时下载内核本体、cmdline 等加载内核所需的文件。

## Build

> 使用 Debian 源的 OVMF 固件未携带 IPv4 网络栈，需要自行编译 edk2

### 依赖

Rust nightly 工具链、GNU binutils（RISC-V / LoongArch）、UEFI 固件、mtools、dosfstools。

```bash
# Debian / Ubuntu
sudo apt install binutils-riscv64-linux-gnu binutils-loongarch64-linux-gnu \
    ovmf qemu-efi-aarch64 qemu-efi-riscv64 qemu-efi-loongarch64 \
    dosfstools mtools
```

### 编译 QEMU

```bash
./configure \
    --target-list=x86_64-softmmu,aarch64-softmmu,riscv64-softmmu,loongarch64-softmmu \
    --enable-sdl --enable-slirp
make -j$(nproc)
```

### 构建 EFI

通过 `ARCH` 变量选择架构，默认 `riscv64`：

```bash
make efi                        # x86_64（默认）
make efi ARCH=aarch64
make efi ARCH=riscv64
make efi ARCH=loongarch64
```

产物在 `target/<arch>/alpheratz.efi`。

x86_64 和 AArch64 有官方 Rust UEFI target，cargo 直接输出 `.efi`。
RISC-V 和 LoongArch 没有官方 UEFI target，先编译为 ELF 再用 GNU objcopy 转换为 PE：

```
cargo (rust-lld) → ELF → objcopy --subsystem efi-app -O pei-*-little → .efi
```

### 构建 Release

```bash
make efi ARCH=riscv64 PROFILE=release
```

### QEMU 测试

```bash
make run                        # x86_64（默认）
make run ARCH=aarch64
make run ARCH=riscv64
make run ARCH=loongarch64
```

### 清理

```bash
make clean
```