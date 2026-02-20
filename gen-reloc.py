#!/usr/bin/env python3
"""
gen-reloc.py — Generate PE base relocations from ELF absolute relocations.

For RISC-V and LoongArch ELF→PE conversion, objcopy does not emit PE base
relocation entries. This script:

  1. Reads R_RISCV_64 / R_LARCH_64 from the ELF's .rela.data section.
  2. Scans .data for additional absolute pointers (GOT entries, vtables)
     that LLD's --emit-relocs misses for synthesized sections.
  3. Patches the PE file with a proper .reloc section.

Usage:
    python3 gen-reloc.py <elf-file> <pe-file>
"""

import struct
import sys

IMAGE_REL_BASED_DIR64 = 0xA
IMAGE_REL_BASED_ABSOLUTE = 0x0
PE_FILE_ALIGNMENT = 0x200
PE_SECTION_ALIGNMENT = 0x1000


def parse_elf_sections(elf_data):
    """Return dict of section name -> {addr, offset, size, type}."""
    u16 = lambda off: struct.unpack_from("<H", elf_data, off)[0]
    u32 = lambda off: struct.unpack_from("<I", elf_data, off)[0]
    u64 = lambda off: struct.unpack_from("<Q", elf_data, off)[0]

    e_shoff = u64(40)
    e_shentsize = u16(58)
    e_shnum = u16(60)
    e_shstrndx = u16(62)

    # section name string table
    strtab_base = e_shoff + e_shstrndx * e_shentsize
    st_off = u64(strtab_base + 24)

    def sec_name(name_off):
        end = elf_data.index(b"\x00", st_off + name_off)
        return elf_data[st_off + name_off : end].decode("ascii")

    sections = {}
    for i in range(e_shnum):
        base = e_shoff + i * e_shentsize
        name = sec_name(u32(base))
        sections[name] = {
            "addr": u64(base + 16),
            "offset": u64(base + 24),
            "size": u64(base + 32),
            "type": u32(base + 4),
            "entsize": u64(base + 56),
        }

    return sections


def read_elf_reloc_offsets(elf_data, sections):
    """Collect offsets needing base relocation from all .rela.* sections."""
    e_machine = struct.unpack_from("<H", elf_data, 18)[0]
    if e_machine not in (0xF3, 0x102):  # EM_RISCV, EM_LOONGARCH
        raise ValueError(f"unsupported e_machine {e_machine:#x}")

    abs_type = 2  # R_RISCV_64 and R_LARCH_64 are both type 2

    offsets = set()
    for name, sec in sections.items():
        if sec["type"] != 4:  # SHT_RELA
            continue
        # Only process .rela.data and .rela.rodata (skip .rela.text — all PC-relative)
        if name not in (".rela.data", ".rela.rodata"):
            continue
        entsize = sec["entsize"] or 24
        raw = elf_data[sec["offset"] : sec["offset"] + sec["size"]]
        for j in range(0, len(raw), entsize):
            r_offset = struct.unpack_from("<Q", raw, j)[0]
            r_info = struct.unpack_from("<Q", raw, j + 8)[0]
            if (r_info & 0xFFFFFFFF) == abs_type:
                offsets.add(r_offset)

    return offsets


def scan_data_for_pointers(elf_data, sections):
    """Scan .data for 8-byte values that are image-internal pointers.

    This catches GOT entries that LLD's --emit-relocs does not cover
    (synthesized GOT slots).  We only scan .data — NOT .rodata, because
    .rodata is full of string literals whose trailing bytes can
    coincidentally look like image addresses (false positives that would
    corrupt data when the UEFI loader applies base relocations).
    """
    data_sec = sections.get(".data")
    if not data_sec:
        return set()

    image_lo = None
    image_hi = 0
    for sec in sections.values():
        if sec["addr"] > 0 and sec["size"] > 0:
            if image_lo is None or sec["addr"] < image_lo:
                image_lo = sec["addr"]
            end = sec["addr"] + sec["size"]
            if end > image_hi:
                image_hi = end

    if image_lo is None:
        return set()

    offsets = set()
    base = data_sec["offset"]
    addr = data_sec["addr"]
    size = data_sec["size"]

    for i in range(0, size & ~7, 8):
        val = struct.unpack_from("<Q", elf_data, base + i)[0]
        if image_lo <= val < image_hi:
            offsets.add(addr + i)

    return offsets


def build_base_reloc_table(offsets):
    """Build PE IMAGE_BASE_RELOCATION blocks for the given RVA list."""
    pages = {}
    for rva in offsets:
        page = rva & ~0xFFF
        pages.setdefault(page, []).append(rva & 0xFFF)

    buf = bytearray()
    for page_rva in sorted(pages):
        entries = pages[page_rva]
        entry_data = bytearray()
        for off in sorted(entries):
            entry_data += struct.pack("<H", (IMAGE_REL_BASED_DIR64 << 12) | off)
        while (8 + len(entry_data)) % 4 != 0:
            entry_data += struct.pack("<H", IMAGE_REL_BASED_ABSOLUTE)
        block_size = 8 + len(entry_data)
        buf += struct.pack("<II", page_rva, block_size)
        buf += entry_data

    return bytes(buf)


def patch_pe(pe_path, reloc_data):
    """Add or replace .reloc section in PE and update Data Directory."""
    with open(pe_path, "rb") as f:
        pe = bytearray(f.read())

    u16 = lambda off: struct.unpack_from("<H", pe, off)[0]
    u32 = lambda off: struct.unpack_from("<I", pe, off)[0]

    pe_sig_off = u32(0x3C)
    if pe[pe_sig_off : pe_sig_off + 4] != b"PE\x00\x00":
        raise ValueError("invalid PE signature")

    coff_off = pe_sig_off + 4
    num_sections = u16(coff_off + 2)
    opt_hdr_size = u16(coff_off + 16)
    coff_chars_off = coff_off + 18

    opt_off = coff_off + 20
    if u16(opt_off) != 0x20B:
        raise ValueError("not PE32+")

    size_of_image_off = opt_off + 56
    size_of_headers_off = opt_off + 60
    data_dir_off = opt_off + 112
    dd5_off = data_dir_off + 5 * 8

    sec_table_off = opt_off + opt_hdr_size

    sections = []
    reloc_idx = None
    for i in range(num_sections):
        sh_off = sec_table_off + i * 40
        name = pe[sh_off : sh_off + 8].rstrip(b"\x00").decode("ascii", errors="replace")
        s = {
            "idx": i,
            "name": name,
            "header_off": sh_off,
            "virtual_size": u32(sh_off + 8),
            "virtual_address": u32(sh_off + 12),
            "raw_size": u32(sh_off + 16),
            "raw_offset": u32(sh_off + 20),
        }
        sections.append(s)
        if name == ".reloc":
            reloc_idx = i

    if reloc_idx is not None:
        sec = sections[reloc_idx]
        if len(reloc_data) <= sec["raw_size"]:
            ro = sec["raw_offset"]
            pe[ro : ro + len(reloc_data)] = reloc_data
            pe[ro + len(reloc_data) : ro + sec["raw_size"]] = b"\x00" * (
                sec["raw_size"] - len(reloc_data)
            )
            struct.pack_into("<I", pe, sec["header_off"] + 8, len(reloc_data))
            struct.pack_into("<I", pe, dd5_off, sec["virtual_address"])
            struct.pack_into("<I", pe, dd5_off + 4, len(reloc_data))
            chars = u16(coff_chars_off)
            struct.pack_into("<H", pe, coff_chars_off, chars & ~0x0001)
            with open(pe_path, "wb") as f:
                f.write(pe)
            return

    # Add a new .reloc section
    raw_size = (len(reloc_data) + PE_FILE_ALIGNMENT - 1) & ~(PE_FILE_ALIGNMENT - 1)

    last = max(sections, key=lambda s: s["virtual_address"])
    last_va_end = last["virtual_address"] + max(last["virtual_size"], last["raw_size"])
    new_rva = (last_va_end + PE_SECTION_ALIGNMENT - 1) & ~(PE_SECTION_ALIGNMENT - 1)
    new_raw_off = (len(pe) + PE_FILE_ALIGNMENT - 1) & ~(PE_FILE_ALIGNMENT - 1)

    headers_end = sec_table_off + num_sections * 40
    size_of_headers = u32(size_of_headers_off)
    if headers_end + 40 > size_of_headers:
        raise ValueError(
            "no room in PE header area for new section "
            f"(headers_end={headers_end:#x}, SizeOfHeaders={size_of_headers:#x})"
        )

    new_sh_off = sec_table_off + num_sections * 40
    header = bytearray(40)
    header[0:8] = b".reloc\x00\x00"
    struct.pack_into("<I", header, 8, len(reloc_data))
    struct.pack_into("<I", header, 12, new_rva)
    struct.pack_into("<I", header, 16, raw_size)
    struct.pack_into("<I", header, 20, new_raw_off)
    struct.pack_into("<I", header, 36, 0x42000040)
    pe[new_sh_off : new_sh_off + 40] = header

    struct.pack_into("<H", pe, coff_off + 2, num_sections + 1)
    new_image_size = new_rva + PE_SECTION_ALIGNMENT
    struct.pack_into("<I", pe, size_of_image_off, new_image_size)
    struct.pack_into("<I", pe, dd5_off, new_rva)
    struct.pack_into("<I", pe, dd5_off + 4, len(reloc_data))

    chars = u16(coff_chars_off)
    struct.pack_into("<H", pe, coff_chars_off, chars & ~0x0001)

    if len(pe) < new_raw_off:
        pe += b"\x00" * (new_raw_off - len(pe))
    pe += reloc_data
    pe += b"\x00" * (raw_size - len(reloc_data))

    with open(pe_path, "wb") as f:
        f.write(pe)


def main():
    if len(sys.argv) != 3:
        print(f"usage: {sys.argv[0]} <elf-file> <pe-file>", file=sys.stderr)
        sys.exit(1)

    elf_path, pe_path = sys.argv[1], sys.argv[2]

    with open(elf_path, "rb") as f:
        elf_data = f.read()

    if elf_data[:4] != b"\x7fELF" or elf_data[4] != 2 or elf_data[5] != 1:
        raise ValueError("not a 64-bit little-endian ELF file")

    sections = parse_elf_sections(elf_data)

    # 1) Offsets from explicit R_*_64 relocations in .rela.data
    reloc_offsets = read_elf_reloc_offsets(elf_data, sections)

    # 2) Additional offsets found by scanning .data for image-internal pointers
    #    (catches synthesized GOT entries that --emit-relocs misses).
    #    We do NOT scan .rodata — string tails create false positives.
    scan_offsets = scan_data_for_pointers(elf_data, sections)

    all_offsets = sorted(reloc_offsets | scan_offsets)
    extra = len(all_offsets) - len(reloc_offsets)

    if not all_offsets:
        print("no absolute relocations found — .reloc not needed")
        return

    reloc_data = build_base_reloc_table(all_offsets)
    patch_pe(pe_path, reloc_data)
    print(
        f"patched {pe_path}: {len(all_offsets)} relocations "
        f"({len(reloc_offsets)} from .rela + {extra} from scan), "
        f"{len(reloc_data)} bytes of .reloc data"
    )


if __name__ == "__main__":
    main()
