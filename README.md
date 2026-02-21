![Canicula Aplheratz](https://picture.hanbings.com/2026/02/21/70fa8c368b2c12e71e2f195c673c38e4ae77db8c754c10c756403795b8d39c3b.png)

<h1 align="center">🌟 Alpheratz</h1>

Canicula OS 的 UEFI Bootloader 应用，用于加载 Linux EFI Stub 和 Canicula OS。

支持 x86_64、AArch64、RISC-V 64、LoongArch64 四个架构，支持基于 UEFI 的网络驱动在启动时下载内核本体、cmdline 等加载内核所需的文件。

## Build

> Debian 源的 OVMF 固件默认不携带完整 IPv4 / HTTP 网络栈，需要自行编译 edk2。
> 编译好的固件放在 `fw/` 目录下，Makefile 会自动引用。

### 依赖

Rust nightly 工具链、GNU binutils（RISC-V / LoongArch）、mtools、dosfstools。

```bash
# Debian / Ubuntu
sudo apt install binutils-riscv64-linux-gnu binutils-loongarch64-linux-gnu \
    gcc-aarch64-linux-gnu gcc-riscv64-linux-gnu gcc-loongarch64-linux-gnu \
    dosfstools mtools qemu-system-x86 qemu-system-arm qemu-system-misc
```

---

### 编译 EDK2 固件（带网络栈）

Debian 源的 OVMF / QEMU_EFI 固件缺少 Ip4Dxe、Dhcp4Dxe、Tcp4Dxe、HttpDxe 等网络驱动，
且 MnpDxe 依赖 `gEfiRngProtocolGuid`（需要 QEMU 提供 `virtio-rng-pci` 设备）。
以下步骤从源码编译包含完整网络栈的固件。

#### 1. 安装编译依赖

```bash
sudo apt install build-essential uuid-dev iasl nasm python3-venv \
    gcc-aarch64-linux-gnu gcc-riscv64-linux-gnu gcc-loongarch64-linux-gnu
```

#### 2. 克隆 edk2

```bash
git clone --depth 1 https://github.com/tianocore/edk2.git
cd edk2
git submodule update --init --depth 1
```

#### 3. 编译 BaseTools

```bash
make -C BaseTools -j$(nproc)
```

#### 4. 编译各架构固件

每次编译前需要 source 环境：

```bash
. edksetup.sh
```

公共编译参数（启用 IPv4 + HTTP + TLS + 允许 HTTP 明文连接）：

```
-b RELEASE \
-D NETWORK_HTTP_BOOT_ENABLE=TRUE \
-D NETWORK_TLS_ENABLE=TRUE \
-D NETWORK_ALLOW_HTTP_CONNECTIONS=TRUE \
-n $(nproc)
```

**x86_64**

```bash
build -a X64 -t GCC5 -p OvmfPkg/OvmfPkgX64.dsc \
    -b RELEASE \
    -D NETWORK_HTTP_BOOT_ENABLE=TRUE \
    -D NETWORK_TLS_ENABLE=TRUE \
    -D NETWORK_ALLOW_HTTP_CONNECTIONS=TRUE \
    -n $(nproc)
```

产物：

```
Build/OvmfX64/RELEASE_GCC5/FV/OVMF_CODE.fd  →  fw/OVMF_X64_CODE.fd
Build/OvmfX64/RELEASE_GCC5/FV/OVMF_VARS.fd  →  fw/OVMF_X64_VARS.fd
```

**AArch64**

```bash
GCC5_AARCH64_PREFIX=aarch64-linux-gnu- \
build -a AARCH64 -t GCC5 -p ArmVirtPkg/ArmVirtQemu.dsc \
    -b RELEASE \
    -D NETWORK_HTTP_BOOT_ENABLE=TRUE \
    -D NETWORK_TLS_ENABLE=TRUE \
    -D NETWORK_ALLOW_HTTP_CONNECTIONS=TRUE \
    -n $(nproc)
```

产物：

```
Build/ArmVirtQemu-AArch64/RELEASE_GCC5/FV/QEMU_EFI.fd   →  fw/QEMU_EFI_AA64.fd
Build/ArmVirtQemu-AArch64/RELEASE_GCC5/FV/QEMU_VARS.fd  →  fw/QEMU_VARS_AA64.fd
```

AArch64 固件需要填充到 64MB 供 QEMU pflash 使用：

```bash
truncate -s 64M fw/QEMU_EFI_AA64.fd
truncate -s 64M fw/QEMU_VARS_AA64.fd
```

**RISC-V 64**

```bash
GCC5_RISCV64_PREFIX=riscv64-linux-gnu- \
build -a RISCV64 -t GCC5 -p OvmfPkg/RiscVVirt/RiscVVirtQemu.dsc \
    -b RELEASE \
    -D NETWORK_HTTP_BOOT_ENABLE=TRUE \
    -D NETWORK_TLS_ENABLE=TRUE \
    -D NETWORK_ALLOW_HTTP_CONNECTIONS=TRUE \
    -n $(nproc)
```

产物：

```
Build/RiscVVirtQemu/RELEASE_GCC5/FV/RISCV_VIRT_CODE.fd  →  fw/RISCV_VIRT_CODE.fd
Build/RiscVVirtQemu/RELEASE_GCC5/FV/RISCV_VIRT_VARS.fd  →  fw/RISCV_VIRT_VARS.fd
```

RISC-V 固件需要填充到 32MB：

```bash
truncate -s 32M fw/RISCV_VIRT_CODE.fd
truncate -s 32M fw/RISCV_VIRT_VARS.fd
```

**LoongArch64**

```bash
GCC5_LOONGARCH64_PREFIX=loongarch64-linux-gnu- \
build -a LOONGARCH64 -t GCC5 -p OvmfPkg/LoongArchVirt/LoongArchVirtQemu.dsc \
    -b RELEASE \
    -D NETWORK_HTTP_BOOT_ENABLE=TRUE \
    -D NETWORK_TLS_ENABLE=TRUE \
    -D NETWORK_ALLOW_HTTP_CONNECTIONS=TRUE \
    -n $(nproc)
```

产物：

```
Build/LoongArchVirtQemu/RELEASE_GCC5/FV/QEMU_EFI.fd   →  fw/QEMU_EFI_LA64.fd
Build/LoongArchVirtQemu/RELEASE_GCC5/FV/QEMU_VARS.fd  →  fw/QEMU_VARS_LA64.fd
```

LoongArch64 固件需要填充到 16MB：

```bash
truncate -s 16M fw/QEMU_EFI_LA64.fd
truncate -s 16M fw/QEMU_VARS_LA64.fd
```

#### 5. 复制到项目

```bash
mkdir -p fw/

# x86_64
cp Build/OvmfX64/RELEASE_GCC5/FV/OVMF_CODE.fd  fw/OVMF_X64_CODE.fd
cp Build/OvmfX64/RELEASE_GCC5/FV/OVMF_VARS.fd  fw/OVMF_X64_VARS.fd

# AArch64
cp Build/ArmVirtQemu-AArch64/RELEASE_GCC5/FV/QEMU_EFI.fd   fw/QEMU_EFI_AA64.fd
cp Build/ArmVirtQemu-AArch64/RELEASE_GCC5/FV/QEMU_VARS.fd  fw/QEMU_VARS_AA64.fd
truncate -s 64M fw/QEMU_EFI_AA64.fd fw/QEMU_VARS_AA64.fd

# RISC-V
cp Build/RiscVVirtQemu/RELEASE_GCC5/FV/RISCV_VIRT_CODE.fd  fw/RISCV_VIRT_CODE.fd
cp Build/RiscVVirtQemu/RELEASE_GCC5/FV/RISCV_VIRT_VARS.fd  fw/RISCV_VIRT_VARS.fd
truncate -s 32M fw/RISCV_VIRT_CODE.fd fw/RISCV_VIRT_VARS.fd

# LoongArch64
cp Build/LoongArchVirtQemu/RELEASE_GCC5/FV/QEMU_EFI.fd   fw/QEMU_EFI_LA64.fd
cp Build/LoongArchVirtQemu/RELEASE_GCC5/FV/QEMU_VARS.fd  fw/QEMU_VARS_LA64.fd
truncate -s 16M fw/QEMU_EFI_LA64.fd fw/QEMU_VARS_LA64.fd
```

#### pflash 大小速查

| 架构 | DSC 文件 | CODE 文件 | VARS 文件 | pflash 大小 |
|------|----------|-----------|-----------|-------------|
| x86_64 | `OvmfPkgX64.dsc` | `OVMF_X64_CODE.fd` | `OVMF_X64_VARS.fd` | 原始大小即可 |
| AArch64 | `ArmVirtQemu.dsc` | `QEMU_EFI_AA64.fd` | `QEMU_VARS_AA64.fd` | 64MB |
| RISC-V | `RiscVVirtQemu.dsc` | `RISCV_VIRT_CODE.fd` | `RISCV_VIRT_VARS.fd` | 32MB |
| LoongArch64 | `LoongArchVirtQemu.dsc` | `QEMU_EFI_LA64.fd` | `QEMU_VARS_LA64.fd` | 16MB |

#### QEMU 注意事项

网络栈中的 MnpDxe 依赖 `gEfiRngProtocolGuid`，需要 QEMU 提供 RNG 设备：

- x86_64 / LoongArch64: `-device virtio-rng-pci`
- AArch64 / RISC-V: `-device virtio-rng-device`

Makefile 的 `QEMU_NET` 变量已包含此设备。

---

### 构建 EFI

通过 `ARCH` 变量选择架构，默认 `x86_64`：

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