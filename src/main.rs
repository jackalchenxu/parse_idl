use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::{fs::File, path::Path};

use anchor_idl::{Idl, IdlType};
use anyhow::anyhow;
use heck::{ToSnakeCase, ToUpperCamelCase};
use log::warn;

fn main() -> anyhow::Result<()> {
    let files = find_idl_json(Path::new("./"))?;

    for fullpath in files {
        let file_name = fullpath.file_stem().unwrap().to_os_string();
        let file_name = file_name.to_str().unwrap();

        let f = File::open(fullpath).unwrap();
        let idl: Idl = serde_json::from_reader(f).unwrap();
        let mut output = File::create(format!("./src/{}.rs", file_name)).unwrap();
        let mut unresolved = HashSet::new();

        add_imports(&mut output);

        let Some(metadata) = idl.metadata else {
            return Err(anyhow!("metadata cannot be None!"));
        };
        let Some(address) = metadata.get("address") else {
            return Err(anyhow!("metadata should contain 'address'"));
        };
        let Some(id) = address.as_str() else {
            return Err(anyhow!("address in metadata should be string format"));
        };

        add_program_id(&mut output, id);

        define_discriminator(&mut output);

        // handle ix method and args
        for ix in idl.instructions.iter() {
            add_discriminator(
                &mut output,
                build_sighash(&ix.name),
                &ix.name.to_snake_case(),
            );
        }
        close_define_discriminator(&mut output);

        // output ix args definition
        for ix in idl.instructions {
            if !ix.args.is_empty() {
                define_struct_or_enum(
                    &mut output,
                    &ix.name.as_str().to_upper_camel_case(),
                    "struct",
                );

                for arg in ix.args {
                    add_struct_field(
                        &mut output,
                        &arg.name.as_str().to_snake_case(),
                        &ty_to_rust_type(&arg.ty, &mut unresolved),
                    );
                }
                close_define_struct_or_enum(&mut output);
            }
        }

        // idl accounts types
        for custom_type in idl.accounts {
            if unresolved.contains(&custom_type.name) {
                match custom_type.ty {
                    anchor_idl::IdlTypeDefinitionTy::Struct { fields } => {
                        define_struct_or_enum(&mut output, custom_type.name.as_str(), "struct");
                        for field in fields.iter() {
                            add_struct_field(
                                &mut output,
                                &field.name.as_str().to_snake_case(),
                                &ty_to_rust_type(&field.ty, &mut unresolved),
                            );
                        }
                        close_define_struct_or_enum(&mut output);
                    }
                    anchor_idl::IdlTypeDefinitionTy::Enum { variants } => {
                        define_struct_or_enum(&mut output, custom_type.name.as_str(), "enum");
                        for field in variants.iter() {
                            add_enum_field(&mut output, field.name.as_str());
                        }
                        close_define_struct_or_enum(&mut output);
                    }
                }
                unresolved.remove(&custom_type.name);
            }
        }

        // idl custome types
        for custom_type in idl.types {
            if unresolved.contains(&custom_type.name) {
                match custom_type.ty {
                    anchor_idl::IdlTypeDefinitionTy::Struct { fields } => {
                        define_struct_or_enum(&mut output, custom_type.name.as_str(), "struct");
                        for field in fields.iter() {
                            add_struct_field(
                                &mut output,
                                &field.name.as_str().to_snake_case(),
                                &ty_to_rust_type(&field.ty, &mut unresolved),
                            );
                        }
                        close_define_struct_or_enum(&mut output);
                    }
                    anchor_idl::IdlTypeDefinitionTy::Enum { variants } => {
                        define_struct_or_enum(&mut output, custom_type.name.as_str(), "enum");
                        for field in variants.iter() {
                            add_enum_field(&mut output, field.name.as_str());
                        }
                        close_define_struct_or_enum(&mut output);
                    }
                }
                unresolved.remove(&custom_type.name);
            }
        }

        for unresolved in unresolved.iter() {
            warn!("resolved type: {}", unresolved);
        }
    }

    Ok(())
}

fn find_idl_json(root_path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut v = vec![];

    for entry in root_path.read_dir()? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            let p = entry.path();
            if let Some(e) = p.extension() {
                if e == "json" {
                    v.push(entry.path());
                }
            }
        }
    }

    Ok(v)
}

fn build_sighash(fname: &str) -> [u8; 8] {
    let function_name = &fname.to_snake_case();

    let mut sighash = [0u8; 8];
    let preimage = format!("global:{}", function_name);

    let mut hasher = openssl::sha::Sha256::new();
    hasher.update(preimage.as_bytes());
    let result = hasher.finish();

    sighash.copy_from_slice(&result.as_slice()[..8]);
    sighash
}

fn add_imports(output: &mut File) {
    output
        .write_all(b"use std::collections::HashMap;\n")
        .unwrap();
    output.write_all(b"use anchor_lang::prelude::*;\n").unwrap();
    output
        .write_all(b"use borsh::{BorshDeserialize, BorshSerialize};\n\n")
        .unwrap();
}

fn add_program_id(output: &mut File, id: &str) {
    output
        .write_fmt(format_args!("static ID: &str = \"{}\";\n", id))
        .unwrap();
}

fn define_discriminator(output: &mut File) {
    output
        .write_all(
            br#"
    pub struct Discriminator(pub HashMap<[u8; 8], String>);
    impl Discriminator {
        pub fn new() -> Self {
            let mut h = HashMap::new();
            "#,
        )
        .unwrap();
}
fn add_discriminator(output: &mut File, bytes: [u8; 8], ix_name: &str) {
    output
        .write_fmt(format_args!(
            "h.insert({:?},\"{}\".to_string());\n",
            bytes, ix_name
        ))
        .unwrap();
}
fn close_define_discriminator(output: &mut File) {
    output
        .write_all(
            br#"Self(h)
        }
    }
    "#,
        )
        .unwrap();
}

fn define_struct_or_enum(output: &mut File, name: &str, type_str: &str) {
    output
        .write_fmt(format_args!(
            "#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]\npub {} {} {{\n",
            type_str, name
        ))
        .unwrap();
}

fn add_struct_field(output: &mut File, field_name: &str, field_type: &str) {
    output
        .write_fmt(format_args!("\t{}: {},\n", field_name, field_type))
        .unwrap()
}
fn add_enum_field(output: &mut File, field_name: &str) {
    output
        .write_fmt(format_args!("\t{},\n", field_name))
        .unwrap()
}

fn close_define_struct_or_enum(output: &mut File) {
    output.write_all(b"}\n").unwrap()
}
pub fn ty_to_rust_type(ty: &IdlType, unresolved: &mut HashSet<String>) -> String {
    match ty {
        IdlType::Bool => "bool".to_string(),
        IdlType::U8 => "u8".to_string(),
        IdlType::I8 => "i8".to_string(),
        IdlType::U16 => "u16".to_string(),
        IdlType::I16 => "i16".to_string(),
        IdlType::U32 => "u32".to_string(),
        IdlType::I32 => "i32".to_string(),
        IdlType::F32 => "f32".to_string(),
        IdlType::U64 => "u64".to_string(),
        IdlType::I64 => "i64".to_string(),
        IdlType::F64 => "f64".to_string(),
        IdlType::U128 => "u128".to_string(),
        IdlType::I128 => "i128".to_string(),
        IdlType::Bytes => "Vec<u8>".to_string(),
        IdlType::String => "String".to_string(),
        IdlType::PublicKey => "Pubkey".to_string(),
        IdlType::Option(inner) => format!("Option<{}>", ty_to_rust_type(inner, unresolved)),
        IdlType::Vec(inner) => format!("Vec<{}>", ty_to_rust_type(inner, unresolved)),
        IdlType::Array(ty, size) => format!("[{}; {}]", ty_to_rust_type(ty, unresolved), size),
        IdlType::Defined(name) => {
            unresolved.insert(name.to_string());
            name.to_string()
        }
    }
}
