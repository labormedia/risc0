// Copyright 2023 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use alloc::{
    vec,
    vec::Vec
};

use anyhow::{Context, Result};
use risc0_zkp::core::{
    digest::Digest,
    hash::sha::{Sha256, BLOCK_BYTES, SHA256_INIT},
};
use risc0_zkvm_platform::{
    memory::{MEM_SIZE, PAGE_TABLE},
    syscall::DIGEST_BYTES,
    WORD_SIZE,
};
use serde::{Deserialize, Serialize};

use crate::{binfmt::elf::Program, sha};

/// Compute `ceil(a / b)` via truncated integer division.
const fn div_ceil(a: u32, b: u32) -> u32 {
    (a + b - 1) / b
}

/// Round `a` up to the nearest multipe of `b`.
const fn round_up(a: u32, b: u32) -> u32 {
    div_ceil(a, b) * b
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PageTableInfo {
    pub page_size: u32,
    page_table_addr: u32,
    _page_table_size: u32,
    root_addr: u32,
    pub root_idx: u32,
    root_page_addr: u32,
    num_pages: u32,
    pub num_root_entries: u32,
    _layers: Vec<u32>,
}

impl PageTableInfo {
    pub fn new(page_table_addr: u32, page_size: u32) -> Self {
        let max_mem = page_table_addr;
        assert!(max_mem >= page_size);

        let mut layers = Vec::new();
        let mut page_table_size = 0u32;
        let mut remain = max_mem;
        while remain >= page_size {
            let num_pages = remain / page_size;
            remain = num_pages * DIGEST_BYTES as u32;
            layers.push(remain);
            page_table_size += remain;
        }
        let max_mem = max_mem + page_table_size;
        let num_pages = max_mem / page_size;
        let page_table_size = round_up(page_table_size, BLOCK_BYTES as u32);
        let root_addr = page_table_addr + page_table_size;
        let root_idx = root_addr / page_size;
        let root_page_addr = root_idx * page_size;
        let num_root_entries = (root_addr - root_page_addr) / DIGEST_BYTES as u32;
        assert_eq!(root_idx, num_pages);

        log::debug!("root_page_addr: 0x{root_page_addr:08x}, root_addr: 0x{root_addr:08x}");

        Self {
            page_size,
            page_table_addr,
            _page_table_size: page_table_size,
            root_addr,
            root_idx,
            root_page_addr,
            num_pages,
            num_root_entries,
            _layers: layers,
        }
    }

    pub fn get_page_addr(&self, page_idx: u32) -> u32 {
        page_idx * self.page_size
    }

    pub fn get_page_index(&self, addr: u32) -> u32 {
        addr / self.page_size
    }

    pub fn get_page_entry_addr(&self, page_idx: u32) -> u32 {
        self.page_table_addr + page_idx * DIGEST_BYTES as u32
    }
}

/// An image of a zkVM guest's memory
///
/// This is an image of the full memory state of the zkVM, including the data,
/// text, inputs, page table, and system memory. In addition to the memory image
/// proper, this includes some metadata about the page table.
#[derive(Clone, Serialize, Deserialize)]
pub struct MemoryImage {
    /// The memory image as a vector of bytes
    pub buf: Vec<u8>,

    /// Metadata about the structure of the page table
    pub info: PageTableInfo,
}

impl MemoryImage {
    /// Construct the initial memory image for `program`
    ///
    /// The result is a MemoryImage with the ELF of `program` loaded (but
    /// execution not yet begun), and with the page table Merkle tree
    /// constructed.
    pub fn new(program: &Program, page_size: u32) -> Result<Self> {
        let mut buf = vec![0_u8; MEM_SIZE];

        // Load the ELF into the memory image.
        for (addr, data) in program.image.iter() {
            let addr = *addr as usize;
            let bytes = data.to_le_bytes();
            buf.get_mut(addr..(WORD_SIZE + addr))
                .context("Invalid Elf Program, address outside MEM_SIZE")?
                .copy_from_slice(&bytes[..WORD_SIZE]);
        }

        // Compute the page table hashes except for the very last root hash.
        let info = PageTableInfo::new(PAGE_TABLE.start() as u32, page_size);
        let mut img = Self { buf, info };
        img.hash_pages();
        Ok(img)
    }

    /// Calculate and update the image merkle tree within this image.
    pub fn hash_pages(&mut self) {
        for i in 0..self.info.num_pages {
            let page_addr = self.info.get_page_addr(i as u32);
            let page =
                &self.buf[page_addr as usize..page_addr as usize + self.info.page_size as usize];
            let digest = hash_page(page);
            let entry_addr = self.info.get_page_entry_addr(i as u32);
            self.buf[entry_addr as usize..entry_addr as usize + DIGEST_BYTES]
                .copy_from_slice(digest.as_bytes());
        }
    }

    /// Verify the integrity of the MemoryImage.
    ///
    /// Confirms that the page table is a valid Merkle tree with the expected
    /// root and that the data from each page hashes to the expected page table
    /// entry.
    #[cfg(test)]
    fn check(&self, addr: u32) -> Result<()> {
        let mut page_idx = self.info.get_page_index(addr);
        while page_idx < self.info.root_idx {
            let page_addr = self.info.get_page_addr(page_idx);
            let page =
                &self.buf[page_addr as usize..page_addr as usize + self.info.page_size as usize];
            let expected = hash_page(page);
            let entry_addr = self.info.get_page_entry_addr(page_idx);
            let entry = &self.buf[entry_addr as usize..entry_addr as usize + DIGEST_BYTES];
            let actual = Digest::try_from(entry)?;
            log::debug!(
                "page_idx: {page_idx}, page_addr: 0x{page_addr:08x} entry_addr: 0x{entry_addr:08x}"
            );
            if expected != actual {
                anyhow::bail!("Invalid page table entry: {} != {}", expected, actual);
            }
            page_idx = self.info.get_page_index(entry_addr);
        }

        let root_page_addr = self.info.root_page_addr;
        let root_page_bytes = self.info.num_root_entries * DIGEST_BYTES as u32;
        let root_page =
            &self.buf[root_page_addr as usize..root_page_addr as usize + root_page_bytes as usize];
        let expected = hash_page(root_page);
        let root = self.get_root();
        if expected != root {
            anyhow::bail!("Invalid root hash: {} != {}", expected, root);
        }

        Ok(())
    }

    /// Compute and return the root entry of the merkle tree.
    pub fn get_root(&self) -> Digest {
        let root_page_addr = self.info.root_page_addr;
        let root_page = &self.buf[root_page_addr as usize..self.info.root_addr as usize];
        hash_page(root_page)
    }
}

fn hash_page(page: &[u8]) -> Digest {
    let mut state = SHA256_INIT;
    assert!(page.len() % BLOCK_BYTES == 0);
    for block in page.chunks_exact(BLOCK_BYTES) {
        let block1 = Digest::try_from(&block[0..DIGEST_BYTES]).unwrap();
        let block2 = Digest::try_from(&block[DIGEST_BYTES..BLOCK_BYTES]).unwrap();
        state = *sha::Impl::compress(&state, &block1, &block2);
    }
    state
}

#[cfg(test)]
mod tests {
    use risc0_zkvm_methods::MULTI_TEST_ELF;
    use risc0_zkvm_platform::{
        memory::{DATA, MEM_SIZE, PAGE_TABLE, STACK, SYSTEM, TEXT},
        syscall::DIGEST_BYTES,
    };
    use test_log::test;

    use super::MemoryImage;
    use crate::binfmt::{elf::Program, image::PageTableInfo};

    fn page_table_size(max_mem: u32, page_size: u32) -> u32 {
        PageTableInfo::new(max_mem, page_size)._page_table_size
    }

    #[test]
    fn check_integrity() {
        const PAGE_SIZE: u32 = 1024;
        let program = Program::load_elf(MULTI_TEST_ELF, TEXT.end() as u32).unwrap();
        let image = MemoryImage::new(&program, PAGE_SIZE).unwrap();
        // This is useful in case one needs to manually inspect the memory image.
        // std::fs::write("/tmp/test.img", &image.image).unwrap();
        image.check(STACK.start() as u32).unwrap();
        image.check(DATA.start() as u32).unwrap();
        image.check(TEXT.start() as u32).unwrap();
        image.check(TEXT.start() as u32 + 5000).unwrap();
        image.check(SYSTEM.start() as u32).unwrap();
        image.check(image.info.root_page_addr).unwrap();
    }

    #[test]
    fn page_table_info() {
        const PAGE_SIZE_1K: u32 = 1024;
        let info = PageTableInfo::new(PAGE_TABLE.start() as u32, PAGE_SIZE_1K);
        assert_eq!(info._page_table_size, 7035584);
        assert_eq!(info._page_table_size / PAGE_SIZE_1K, 6870);
        assert_eq!(info._page_table_size / PAGE_SIZE_1K * PAGE_SIZE_1K, 7034880);
        assert_eq!(info._layers, vec![6815744, 212992, 6656, 192]);
        assert_eq!(info.root_addr, 0xd6b5ac0);
        assert_eq!(info.root_page_addr, 0xd6b5800);
        assert_eq!(info.num_root_entries, 22);
        assert_eq!(info.root_idx, 219862);
    }

    #[test]
    fn page_size_1k() {
        const PAGE_SIZE_1K: u32 = 1024;
        assert_eq!(
            page_table_size(PAGE_SIZE_1K, PAGE_SIZE_1K),
            DIGEST_BYTES as u32 * 2
        );
        assert_eq!(
            page_table_size(PAGE_SIZE_1K * 2, PAGE_SIZE_1K),
            DIGEST_BYTES as u32 * 2
        );
        assert_eq!(
            page_table_size(PAGE_SIZE_1K * 256, PAGE_SIZE_1K),
            DIGEST_BYTES as u32 * 256 + 256
        );
        // max_mem: 256M, page: 1K bytes
        // Layer 1: 256M / 1K = 256K pages => 256K * 32 =   8M
        // Layer 2:   8M / 1K =   8K pages =>   8K * 32 = 256K
        // Layer 3: 256K / 1K =  256 pages =>  256 * 32 =   8K
        // Layer 4:   8K / 1K =    8 pages =>    8 * 32 =  256
        let info = PageTableInfo::new(256 * 1024 * 1024, PAGE_SIZE_1K);
        assert_eq!(
            info._layers,
            vec![8 * 1024 * 1024, 256 * 1024, 8 * 1024, 256]
        );
        assert_eq!(
            info._page_table_size,
            8 * 1024 * 1024 + 256 * 1024 + 8 * 1024 + 256
        );
    }

    #[test]
    fn page_size_4k() {
        const PAGE_SIZE_4K: u32 = 4 * 1024;
        assert_eq!(
            page_table_size(PAGE_SIZE_4K, PAGE_SIZE_4K),
            DIGEST_BYTES as u32 * 2
        );
        assert_eq!(
            page_table_size(PAGE_SIZE_4K * 2, PAGE_SIZE_4K),
            DIGEST_BYTES as u32 * 2
        );
        assert_eq!(
            page_table_size(2 * 1024 * 1024, PAGE_SIZE_4K),
            16 * 1024 + 128
        );
        // max_mem: 256M, page: 4K bytes
        // Layer 1: 256M / 4K =  64K pages =>  64K * 32 =   2M
        // Layer 2:   2M / 4K =  512 pages =>  512 * 32 =  16K
        // Layer 3:  16K / 4K =    4 pages =>    4 * 32 =  128
        let info = PageTableInfo::new(256 * 1024 * 1024, PAGE_SIZE_4K);
        assert_eq!(info._layers, vec![2 * 1024 * 1024, 16 * 1024, 128]);
        assert_eq!(info._page_table_size, 2 * 1024 * 1024 + 16 * 1024 + 128);
    }

    #[test]
    fn page_size_1k_fractional() {
        const PAGE_SIZE_1K: u32 = 1024;

        // max_mem: 6656, page: 1K bytes
        // Layer 1: 6656 / 1K = 6 pages => 6 * 32 = 192

        // 0x0000..0x0400 -> P0
        // 0x0400..0x0800 -> P1
        // 0x0800..0x0C00 -> P2
        // 0x0C00..0x1000 -> P3
        // 0x1000..0x1400 -> P4
        // 0x1400..0x1800 -> P5
        // 0x1800..0x1AC0 -> P6 (fractional)

        // 0x1A00: P0
        // 0x1A20: P1
        // 0x1A40: P2
        // 0x1A60: P3
        // 0x1A80: P4
        // 0x1AA0: P5
        // 0x1AC0: Root

        let info = PageTableInfo::new(0x1A00, PAGE_SIZE_1K);
        assert_eq!(info._layers, vec![192]);
        assert_eq!(info._page_table_size, 192);
        assert_eq!(info.root_addr, 0x1AC0);
        assert_eq!(info.root_page_addr, 0x1800);
        assert_eq!(
            info.num_root_entries,
            (0x1A00 - 0x1800) / DIGEST_BYTES as u32 + 6
        );
    }

    #[test]
    #[should_panic(expected = "Invalid Elf Program, address outside MEM_SIZE")]
    fn test_fuzzing_oob_idx_bug() {
        let data = b"\x7f\x45\x4c\x46\x01\x01\x01\x01\x01\x01\xff\xff\x00\x00\x00\x00\x02\x00\xf3\x00\x00\x00\x00\x00\x00\x00\x01\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x04\x20\x00\x08\x00\x00\x00\x96\x96\x00\x94\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x00\x00\x00\xff\x00\x00\x94\x00\x00\x00\xff\xf6\x12\xa9\x00\x00\x00\x00\x00\x00\xfe\x00\x00\x00\x00\x00\x0a\x9a\x38\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x96\x4c\x46\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00\x00\x00\x04\x00\x00\x00\x00\x00\x00\x00\x02\x00\x00\x00\x00\x0a\x9d\xd8\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x40\x1e\x00\x00\x46\x4c\x00\x00\x00\x00\x00\x02\x00\x40\x00\x01\x01\x01\x00\x04\x00\x00\x00\x00\x00\x01\x00\x00\x00\x00\x05\x00\x00\x07\x78\xc1\x0a\x00\x00\xba\x00\x00\x00\x00\xe3\x04\x00\x00\x31\x35\x32\x37\x38\x31\x46\x01\x01\x01\x01\x01\x01\xff\xff\x00\x00\x00\x00\x02\x00\xe5\x00\x00\x00\x00\x96\x96\x00\xff\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x04\x00\x00\x00\x00\x00\xff\xff\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x0b\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x06\x2e\xac\x00\x00\x00\x00\x00\x00\x0a\xce\x58\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\xff\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00\x00\x00\x00\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x40\x1e\x1e\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00\x40\x00\x00\x00\x00\x00\x04\x00\x00\x00\x00\x00\x00\x05\x00\x00\x07\x00\xba\xe8\xad\x0a\x00\xe3\x04\x00\x00\x00\x00\x12\x00\x00\x00\x00\x08\x00\x00\x00\x00\x00\x00\x00\x00\x00\x05\x00\x00\x05\x00\x00\x00\x01\x01\x01\x50\xcf\x0a\x00\x01\x01\x01\x01\x01\x01\x01\x01\x00\x00\x00\x00\x00\x00\x00\x04\x01\x01\x01\x01\x01\x01\x01\x00\x00\x31\x35\x31\x35\x32\x37\x38\x31\x30\x34\x02\x00\x00\x00\x00\x00\x00\x00\x00\x05\x00\x00\x05\x00\x00\x00\x01\x01\x01\x01\x01\x01\x01\x00\x00\x00\x00\x00\x00\x07\x00\x00\x00\xff\xff\xff\xff\x00\x00\x00\x00\xff\x04\x92\x01\x01\x01\x01\x01\x01\xa2\xf8\x00\x20\x00\x00\x00\x00\xff\x00\x40\x00\x04\x00\x00\x00\x38\x00\x00\x00\x00\x00\x00\x00\x02\x00\x0a\x40\x40\x00\x1a\x00\x19\x00";
        const PAGE_SIZE: u32 = 1024;
        let prog = Program::load_elf(data, MEM_SIZE as u32).unwrap();
        let _res = MemoryImage::new(&prog, PAGE_SIZE).unwrap();
    }
}
