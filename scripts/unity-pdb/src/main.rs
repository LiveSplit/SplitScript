//! Print IL2CPP structure members and bitfields from a player's own PDB.
use pdb::{FallibleIterator, TypeData, TypeFinder, TypeIndex};
use serde_json::{Map, Value, json};
use std::{error::Error, fs::File};

fn members(finder: &TypeFinder<'_>, mut index: TypeIndex) -> Result<Value, Box<dyn Error>> {
    let mut result = Map::new();
    loop {
        let TypeData::FieldList(list) = finder.find(index)?.parse()? else {
            return Err("expected a field list".into());
        };
        for field in list.fields {
            if let TypeData::Member(member) = field {
                let mut value = json!({"offset": member.offset});
                match finder.find(member.field_type)?.parse()? {
                    TypeData::Bitfield(bit) => {
                        value["bit"] = json!(bit.position);
                        value["bits"] = json!(bit.length);
                    }
                    TypeData::Class(class) => value["type"] = json!(class.name.to_string()),
                    _ => {}
                }
                if result
                    .insert(member.name.to_string().into_owned(), value)
                    .is_some()
                {
                    return Err("duplicate field name".into());
                }
            }
        }
        match list.continuation {
            Some(next) => index = next,
            None => return Ok(Value::Object(result)),
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: unity-pdb-layouts GameAssembly.pdb")?;
    let mut pdb = pdb::PDB::open(File::open(path)?)?;
    let types = pdb.type_information()?;
    let mut finder = types.finder();
    let mut iter = types.iter();
    let mut classes = Vec::new();
    while let Some(record) = iter.next()? {
        finder.update(&iter);
        if let Ok(TypeData::Class(class)) = record.parse() {
            let name = class.name.to_string();
            if matches!(
                name.as_ref(),
                "Il2CppClass" | "Il2CppType" | "Il2CppGenericClass"
            ) && !class.properties.forward_reference()
            {
                classes.push((name.into_owned(), class.size, class.fields));
            }
        }
    }
    let mut result = Map::new();
    for (name, size, fields) in classes {
        let fields = members(&finder, fields.ok_or("missing fields")?)?;
        let value = json!({"size": size, "fields": fields});
        if let Some(previous) = result.insert(name.clone(), value.clone())
            && previous != value
        {
            return Err(
                format!("conflicting {name} definitions: {previous} versus {value}").into(),
            );
        }
    }
    if result.len() != 3 {
        return Err("PDB lacks the three IL2CPP type definitions".into());
    }
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
