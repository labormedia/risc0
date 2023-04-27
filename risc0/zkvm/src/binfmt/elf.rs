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

use anyhow::{anyhow, bail, Context};
use core::{
    result::Result,
    fmt::Display,
};
use alloc::{
    boxed::Box,
    collections::BTreeMap,
    string::{
        String,
        ToString,
        ParseError
    },
    fmt::Error
};
use elf::{endian::LittleEndian, file::Class, ElfBytes};

/// A RISC Zero program
pub struct Program {
    /// The entrypoint of the program
    pub entry: u32,

    /// The initial memory image
    pub image: BTreeMap<u32, u32>,
}

#[derive(Debug)]
enum ProgramErrors {
    Error(String),
}

impl Display for ProgramErrors {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ProgramErrors::Error(msg) => 
                write!(f, "{}", msg),
        }
    }
}

impl core::error::Error for ProgramErrors {}

impl Program {
    /// Initialize a RISC Zero Program from an appropriate ELF file
    pub fn load_elf(input: &[u8], max_mem: u32) -> Result<Program, String>
    {
        let mut image: BTreeMap<u32, u32> = BTreeMap::new();
        let elf = ElfBytes::<LittleEndian>::minimal_parse(input).expect("Could not parse");
        if elf.ehdr.class != Class::ELF32 {
            // bail!("Not a 32-bit ELF");
            return Err("Not a 32-bit ELF".to_string())
        }
        if elf.ehdr.e_machine != elf::abi::EM_RISCV {
            // bail!("Invalid machine type, must be RISC-V");
            return Err("Invalid machine type, must be RISC-V".to_string())
        }
        if elf.ehdr.e_type != elf::abi::ET_EXEC {
            // bail!("Invalid ELF type, must be executable");
            return Err("Invalid ELF type, must be executable".to_string())
        }
        let entry: u32 = elf.ehdr.e_entry.try_into().expect("Entry error");
        if entry >= max_mem || entry % 4 != 0 {
            // bail!("Invalid entrypoint");
            return Err("Invalid entrypoint".to_string())
        }
        let segments = elf.segments().expect("Missing segment table");
        if segments.len() > 256 {
            // bail!("Too many program headers");
            return Err("Too many program headers".to_string())
        }
        for segment in segments.iter().filter(|x| x.p_type == elf::abi::PT_LOAD) {
            let file_size: u32 = segment.p_filesz.try_into().expect("Invalid segment.");
            if file_size >= max_mem {
                // bail!("Invalid segment file_size");
                return Err("Invalid segment file_size".to_string())
            }
            let mem_size: u32 = segment.p_memsz.try_into().expect("Memory allocation");
            if mem_size >= max_mem {
                // bail!("Invalid segment mem_size");
                return Err("Invalid segment mem_size".to_string())
            }
            let vaddr: u32 = segment.p_vaddr.try_into().expect("Invalid vaddr.");
            let offset: u32 = segment.p_offset.try_into().expect("Invalid offset.");
            for i in (0..mem_size).step_by(4) {
                let addr = vaddr.checked_add(i).context("Invalid segment vaddr").expect("Invalid segment.");
                if i >= file_size {
                    // Past the file size, all zeros.
                    image.insert(addr, 0);
                } else {
                    let mut word = 0;
                    // Don't read past the end of the file.
                    let len = core::cmp::min(file_size - i, 4);
                    for j in 0..len {
                        let offset = (offset + i + j) as usize;
                        let byte = input.get(offset).context("Invalid segment offset").expect("Invalid offset.");
                        word |= (*byte as u32) << (j * 8);
                    }
                    image.insert(addr, word);
                }
            }
        }
        Ok(Program { entry, image })
    }
}
