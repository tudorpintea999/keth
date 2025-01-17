use crate::model::U128_BYTES_SIZE;
use alloy_primitives::U256;
use cairo_vm::{
    serde::deserialize_program::{Identifier, Location},
    types::{
        errors::math_errors::MathError,
        relocatable::{MaybeRelocatable, Relocatable},
    },
    vm::{errors::memory_errors::MemoryError, runners::cairo_runner::CairoRunner},
    Felt252,
};
use std::collections::HashMap;
use thiserror::Error;

/// Represents errors that can occur during the serialization and deserialization processes between
/// Cairo VM programs and Rust representations.
#[derive(Debug, Error)]
pub enum KakarotSerdeError {
    /// Error variant indicating that no identifier matching the specified name was found.
    #[error("Expected one struct named '{struct_name}', found 0 matches. Expected type: {expected_type:?}")]
    IdentifierNotFound {
        /// The name of the struct that was not found.
        struct_name: String,
        /// The expected type of the struct (if applicable).
        expected_type: Option<String>,
    },

    /// Error variant indicating that multiple identifiers matching the specified name were found.
    #[error("Expected one struct named '{struct_name}', found {count} matches. Expected type: {expected_type:?}")]
    MultipleIdentifiersFound {
        /// The name of the struct for which multiple identifiers were found.
        struct_name: String,
        /// The expected type of the struct (if applicable).
        expected_type: Option<String>,
        /// The number of matching identifiers found.
        count: usize,
    },

    /// Error variant indicating a Math error in CairoVM operations
    #[error(transparent)]
    CairoVmMath(#[from] MathError),

    /// Error variant indicating a memory error in CairoVM operations
    #[error(transparent)]
    CairoVmMemory(#[from] MemoryError),

    /// Error variant indicating that a required field was not found during serialization.
    #[error("Missing required field '{field}' in serialization process.")]
    MissingField {
        /// The name of the missing field.
        field: String,
    },
}

/// Represents the types used in Cairo, including felt types, pointers, tuples, and structs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CairoType {
    /// A felt type, optionally associated with a location.
    Felt { location: Option<Location> },

    /// A pointer type that points to another [`CairoType`], with an optional location.
    Pointer { pointee: Box<CairoType>, location: Option<Location> },

    /// A tuple type that consists of multiple tuple items.
    Tuple { members: Vec<TupleItem>, has_trailing_comma: bool, location: Option<Location> },

    /// A struct type defined by its scope and an optional location.
    Struct { scope: ScopedName, location: Option<Location> },
}

impl CairoType {
    /// Creates a new [`CairoType::Struct`] with the specified scope and optional location.
    pub fn struct_type(scope: &str, location: Option<Location>) -> Self {
        Self::Struct { scope: ScopedName::from_string(scope), location }
    }

    /// Creates a new [`CairoType::Felt`] with an optional location.
    pub fn felt_type(location: Option<Location>) -> Self {
        Self::Felt { location }
    }

    /// Creates a new [`CairoType::Pointer`] that points to a specified [`CairoType`].
    pub fn pointer_type(pointee: CairoType, location: Option<Location>) -> Self {
        Self::Pointer { pointee: Box::new(pointee), location }
    }

    /// Creates a new [`CairoType::Tuple`] from a vector of [`TupleItem`]s.
    pub fn tuple_from_members(
        members: Vec<TupleItem>,
        has_trailing_comma: bool,
        location: Option<Location>,
    ) -> Self {
        Self::Tuple { members, has_trailing_comma, location }
    }
}

/// Represents an item in a tuple, consisting of an optional name, type, and location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TupleItem {
    /// An optional string representing the name of the tuple item.
    pub name: Option<String>,

    /// The [`CairoType`] of the tuple item.
    pub typ: CairoType,

    /// An optional location associated with the tuple item.
    pub location: Option<Location>,
}

impl TupleItem {
    /// Creates a new [`TupleItem`] with an optional name, Cairo type, and location.
    pub fn new(name: Option<String>, typ: CairoType, location: Option<Location>) -> Self {
        Self { name, typ, location }
    }
}

/// Represents a scoped name composed of a series of identifiers forming a path.
///
/// Example: `starkware.cairo.common.uint256.Uint256`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScopedName {
    /// A vector of strings representing the components of the scoped name.
    ///
    /// Each element in the vector corresponds to a segment of the name, separated by
    /// a dot (`.`).
    ///
    /// The first element is the top-level namespace, and subsequent elements represent
    /// sub-namespaces or types. This structure allows for easy manipulation and representation
    /// of names in a hierarchical format.
    pub path: Vec<String>,
}

impl ScopedName {
    /// Separator for the scope path.
    const SEPARATOR: &'static str = ".";

    /// Creates a [`ScopedName`] from a dot-separated string.
    pub fn from_string(scope: &str) -> Self {
        let path = if scope.is_empty() {
            vec![]
        } else {
            scope.split(Self::SEPARATOR).map(String::from).collect()
        };
        Self { path }
    }
}

/// A structure representing the Kakarot serialization and deserialization context for Cairo
/// programs.
///
/// This struct encapsulates the components required to serialize and deserialize
/// Kakarot programs, including:
/// - The Cairo runner responsible for executing the program
#[allow(missing_debug_implementations)]
pub struct KakarotSerde {
    /// The Cairo runner used to execute Kakarot programs.
    ///
    /// This runner interacts with the Cairo virtual machine, providing the necessary
    /// infrastructure for running and managing the execution of Cairo programs.
    /// It is responsible for handling program execution flow, managing state, and
    /// providing access to program identifiers.
    runner: CairoRunner,
}

impl KakarotSerde {
    /// Retrieves a unique identifier from the Cairo program based on the specified struct name and
    /// expected type.
    ///
    /// This function searches for identifiers that match the provided struct name and type within
    /// the Cairo program's identifier mappings. It returns an error if no identifiers or
    /// multiple identifiers are found.
    pub fn get_identifier(
        &self,
        struct_name: &str,
        expected_type: Option<String>,
    ) -> Result<Identifier, KakarotSerdeError> {
        // Retrieve identifiers from the program and filter them based on the struct name and
        // expected type
        let identifiers = self
            .runner
            .get_program()
            .iter_identifiers()
            .filter(|(key, value)| {
                key.contains(struct_name) &&
                    key.split('.').last() == struct_name.split('.').last() &&
                    value.type_ == expected_type
            })
            .map(|(_, value)| value)
            .collect::<Vec<_>>();

        // Match on the number of found identifiers
        match identifiers.len() {
            // No identifiers found
            0 => Err(KakarotSerdeError::IdentifierNotFound {
                struct_name: struct_name.to_string(),
                expected_type,
            }),
            // Exactly one identifier found, return it
            1 => Ok(identifiers[0].clone()),
            // More than one identifier found
            count => Err(KakarotSerdeError::MultipleIdentifiersFound {
                struct_name: struct_name.to_string(),
                expected_type,
                count,
            }),
        }
    }

    /// Serializes a pointer to a Hashmap by resolving its members from memory.
    ///
    /// We provide:
    /// - The name of the struct whose pointer is being serialized.
    /// - The memory location (pointer) of the struct.
    ///
    /// We expect:
    /// - A map of member names to their corresponding values (or `None` if the pointer is 0).
    pub fn serialize_pointers(
        &self,
        struct_name: &str,
        ptr: Relocatable,
    ) -> Result<HashMap<String, Option<MaybeRelocatable>>, KakarotSerdeError> {
        // Fetch the struct definition (identifier) by name.
        let identifier = self.get_identifier(struct_name, Some("struct".to_string()))?;

        // Initialize the output map.
        let mut output = HashMap::new();

        // If the struct has members, iterate over them to resolve their values from memory.
        if let Some(members) = identifier.members {
            for (name, member) in members {
                // We try to resolve the member's value from memory.
                if let Some(member_ptr) = self.runner.vm.get_maybe(&(ptr + member.offset)?) {
                    // Check for null pointer.
                    if member_ptr == MaybeRelocatable::Int(Felt252::ZERO) &&
                        member.cairo_type.ends_with('*')
                    {
                        // We insert `None` for cases such as `parent=cast(0, model.Parent*)`
                        //
                        // Null pointers are represented as `None`.
                        output.insert(name, None);
                    } else {
                        // Insert the resolved member pointer into the output map.
                        output.insert(name, Some(member_ptr));
                    }
                }
            }
        }

        Ok(output)
    }

    /// Serializes a Cairo VM `Uint256` structure (with `low` and `high` fields) into a Rust
    /// [`U256`] value.
    ///
    /// This function retrieves the `Uint256` struct from memory, extracts its `low` and `high`
    /// values, converts them into a big-endian byte representation, and combines them into a
    /// single [`U256`].
    pub fn serialize_uint256(&self, ptr: Relocatable) -> Result<U256, KakarotSerdeError> {
        // Fetches the `Uint256` structure from memory.
        let raw = self.serialize_pointers("Uint256", ptr)?;

        // Retrieves the `low` field from the deserialized struct, ensuring it's a valid integer.
        let low = match raw.get("low") {
            Some(Some(MaybeRelocatable::Int(value))) => value,
            _ => return Err(KakarotSerdeError::MissingField { field: "low".to_string() }),
        };

        // Retrieves the `high` field from the deserialized struct, ensuring it's a valid integer.
        let high = match raw.get("high") {
            Some(Some(MaybeRelocatable::Int(value))) => value,
            _ => return Err(KakarotSerdeError::MissingField { field: "high".to_string() }),
        };

        // Converts the `low` and `high` values into big-endian byte arrays.
        let high_bytes = high.to_bytes_be();
        let low_bytes = low.to_bytes_be();

        // Concatenates the last 16 bytes (128 bits) of the `high` and `low` byte arrays.
        //
        // This forms a 256-bit number, where:
        // - The `high` bytes make up the most significant 128 bits
        // - The `low` bytes make up the least significant 128 bits.
        let bytes = [&high_bytes[U128_BYTES_SIZE..], &low_bytes[U128_BYTES_SIZE..]].concat();

        // Creates a `U256` value from the concatenated big-endian byte array.
        Ok(U256::from_be_slice(&bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairo_vm::{
        serde::deserialize_program::InputFile,
        types::{layout_name::LayoutName, program::Program},
    };
    use std::str::FromStr;

    fn setup_kakarot_serde() -> KakarotSerde {
        // Load the valid program content from a JSON file
        let program_content = include_bytes!("../testdata/keccak_add_uint256.json");

        // Create a Program instance from the loaded bytes, specifying "main" as the entry point
        let program = Program::from_bytes(program_content, Some("main")).unwrap();

        // Initialize a CairoRunner with the created program and default parameters
        let runner = CairoRunner::new(&program, LayoutName::plain, false, false).unwrap();

        // Return an instance of KakarotSerde
        KakarotSerde { runner }
    }

    #[test]
    fn test_program_identifier_valid() {
        // Setup the KakarotSerde instance
        let kakarot_serde = setup_kakarot_serde();

        // Check if the identifier "main" with expected type "function" is correctly retrieved
        assert_eq!(
            kakarot_serde.get_identifier("main", Some("function".to_string())).unwrap(),
            Identifier {
                pc: Some(96),
                type_: Some("function".to_string()),
                value: None,
                full_name: None,
                members: None,
                cairo_type: None
            }
        );

        // Check if the identifier "__temp0" with expected type "reference" is correctly retrieved
        assert_eq!(
            kakarot_serde.get_identifier("__temp0", Some("reference".to_string())).unwrap(),
            Identifier {
                pc: None,
                type_: Some("reference".to_string()),
                value: None,
                full_name: Some(
                    "starkware.cairo.common.uint256.word_reverse_endian.__temp0".to_string()
                ),
                members: None,
                cairo_type: Some("felt".to_string())
            }
        );
    }

    #[test]
    fn test_non_existent_identifier() {
        // Setup the KakarotSerde instance
        let kakarot_serde = setup_kakarot_serde();

        // Test for a non-existent identifier
        let result =
            kakarot_serde.get_identifier("non_existent_struct", Some("function".to_string()));

        // Check if the error is valid and validate its parameters
        if let Err(KakarotSerdeError::IdentifierNotFound { struct_name, expected_type }) = result {
            assert_eq!(struct_name, "non_existent_struct");
            assert_eq!(expected_type, Some("function".to_string()));
        } else {
            panic!("Expected KakarotSerdeError::IdentifierNotFound");
        }
    }

    #[test]
    fn test_incorrect_identifier_usage() {
        // Setup the KakarotSerde instance
        let kakarot_serde = setup_kakarot_serde();

        // Test for an identifier used incorrectly (not the last segment of the full name)
        let result = kakarot_serde.get_identifier("check_range", Some("struct".to_string()));

        // Check if the error is valid and validate its parameters
        if let Err(KakarotSerdeError::IdentifierNotFound { struct_name, expected_type }) = result {
            assert_eq!(struct_name, "check_range");
            assert_eq!(expected_type, Some("struct".to_string()));
        } else {
            panic!("Expected KakarotSerdeError::IdentifierNotFound");
        }
    }

    #[test]
    fn test_valid_identifier_incorrect_type() {
        // Setup the KakarotSerde instance
        let kakarot_serde = setup_kakarot_serde();

        // Test for a valid identifier but with an incorrect type
        let result = kakarot_serde.get_identifier("main", Some("struct".to_string()));

        // Check if the error is valid and validate its parameters
        if let Err(KakarotSerdeError::IdentifierNotFound { struct_name, expected_type }) = result {
            assert_eq!(struct_name, "main");
            assert_eq!(expected_type, Some("struct".to_string()));
        } else {
            panic!("Expected KakarotSerdeError::IdentifierNotFound");
        }
    }

    #[test]
    fn test_identifier_with_multiple_matches() {
        // Setup the KakarotSerde instance
        let kakarot_serde = setup_kakarot_serde();

        // Test for an identifier with multiple matches
        let result = kakarot_serde.get_identifier("ImplicitArgs", Some("struct".to_string()));

        // Check if the error is valid and validate its parameters
        if let Err(KakarotSerdeError::MultipleIdentifiersFound {
            struct_name,
            expected_type,
            count,
        }) = result
        {
            assert_eq!(struct_name, "ImplicitArgs");
            assert_eq!(expected_type, Some("struct".to_string()));
            assert_eq!(count, 6);
        } else {
            panic!("Expected KakarotSerdeError::MultipleIdentifiersFound");
        }
    }

    #[test]
    fn test_serialize_pointer_not_struct() {
        // Setup the KakarotSerde instance
        let mut kakarot_serde = setup_kakarot_serde();

        // Add a new memory segment to the virtual machine (VM).
        let base = kakarot_serde.runner.vm.add_memory_segment();

        // Attempt to serialize pointer with "main", expecting an IdentifierNotFound error.
        let result = kakarot_serde.serialize_pointers("main", base);

        // Assert that the result is an error with the expected struct name and type.
        match result {
            Err(KakarotSerdeError::IdentifierNotFound { struct_name, expected_type }) => {
                assert_eq!(struct_name, "main".to_string());
                assert_eq!(expected_type, Some("struct".to_string()));
            }
            _ => panic!("Expected KakarotSerdeError::IdentifierNotFound, but got: {:?}", result),
        }
    }

    #[test]
    fn test_serialize_pointer_empty() {
        // Setup the KakarotSerde instance
        let kakarot_serde = setup_kakarot_serde();

        // Serialize the pointers of the "ImplicitArgs" struct but without any memory segment.
        let result = kakarot_serde
            .serialize_pointers("main.ImplicitArgs", Relocatable::default())
            .expect("failed to serialize pointers");

        // The result should be an empty HashMap since there is no memory segment.
        assert!(result.is_empty(),);
    }

    #[test]
    fn test_serialize_pointer_valid() {
        // Setup the KakarotSerde instance
        let mut kakarot_serde = setup_kakarot_serde();

        // Setup
        let output_ptr = Felt252::ZERO;
        let range_check_ptr = kakarot_serde.runner.vm.add_memory_segment();
        let bitwise_ptr = kakarot_serde.runner.vm.add_memory_segment();

        // Insert values in memory
        let base = kakarot_serde
            .runner
            .vm
            .gen_arg(&vec![
                MaybeRelocatable::Int(output_ptr),
                MaybeRelocatable::RelocatableValue(range_check_ptr),
                MaybeRelocatable::RelocatableValue(bitwise_ptr),
            ])
            .unwrap()
            .get_relocatable()
            .unwrap();

        // Serialize the pointers of the "ImplicitArgs" struct using the new memory segment.
        let result = kakarot_serde
            .serialize_pointers("main.ImplicitArgs", base)
            .expect("failed to serialize pointers");

        // Assert that the result matches the expected serialized struct members.
        assert_eq!(
            result,
            HashMap::from_iter([
                ("output_ptr".to_string(), None),
                (
                    "range_check_ptr".to_string(),
                    Some(MaybeRelocatable::RelocatableValue(range_check_ptr))
                ),
                ("bitwise_ptr".to_string(), Some(MaybeRelocatable::RelocatableValue(bitwise_ptr))),
            ])
        );
    }

    #[test]
    fn test_serialize_null_no_pointer() {
        // Setup the KakarotSerde instance
        let mut kakarot_serde = setup_kakarot_serde();

        // Setup
        let output_ptr = Relocatable { segment_index: 10, offset: 11 };
        let range_check_ptr = Felt252::ZERO;
        let bitwise_ptr = Felt252::from(55);

        // Insert values in memory
        let base = kakarot_serde
            .runner
            .vm
            .gen_arg(&vec![
                MaybeRelocatable::RelocatableValue(output_ptr),
                MaybeRelocatable::Int(range_check_ptr),
                MaybeRelocatable::Int(bitwise_ptr),
            ])
            .unwrap()
            .get_relocatable()
            .unwrap();

        // Serialize the pointers of the "ImplicitArgs" struct using the new memory segment.
        let result = kakarot_serde
            .serialize_pointers("main.ImplicitArgs", base)
            .expect("failed to serialize pointers");

        // Assert that the result matches the expected serialized struct members.
        assert_eq!(
            result,
            HashMap::from_iter([
                ("output_ptr".to_string(), Some(MaybeRelocatable::RelocatableValue(output_ptr))),
                // Not a pointer so that we shouldn't have a `None`
                ("range_check_ptr".to_string(), Some(MaybeRelocatable::Int(range_check_ptr))),
                ("bitwise_ptr".to_string(), Some(MaybeRelocatable::Int(bitwise_ptr))),
            ])
        );
    }

    #[test]
    fn test_serialize_uint256_0() {
        // Setup the KakarotSerde instance
        let mut kakarot_serde = setup_kakarot_serde();

        // U256 to be serialized
        let x = U256::ZERO;

        // Setup with the high and low parts of the U256
        let low =
            Felt252::from_bytes_be_slice(&x.to_be_bytes::<{ U256::BYTES }>()[U128_BYTES_SIZE..]);
        let high =
            Felt252::from_bytes_be_slice(&x.to_be_bytes::<{ U256::BYTES }>()[0..U128_BYTES_SIZE]);

        // Insert values in memory
        let base = kakarot_serde
            .runner
            .vm
            .gen_arg(&vec![MaybeRelocatable::Int(low), MaybeRelocatable::Int(high)])
            .unwrap()
            .get_relocatable()
            .unwrap();

        // Serialize the Uint256 struct using the new memory segment.
        let result = kakarot_serde.serialize_uint256(base).expect("failed to serialize pointers");

        // Assert that the result is 0.
        assert_eq!(result, U256::ZERO);
    }

    #[test]
    fn test_serialize_uint256_valid() {
        // Setup the KakarotSerde instance
        let mut kakarot_serde = setup_kakarot_serde();

        // U256 to be serialized
        let x =
            U256::from_str("0x52f8f61201b2b11a78d6e866abc9c3db2ae8631fa656bfe5cb53668255367afb")
                .unwrap();

        // Setup with the high and low parts of the U256
        let low =
            Felt252::from_bytes_be_slice(&x.to_be_bytes::<{ U256::BYTES }>()[U128_BYTES_SIZE..]);
        let high =
            Felt252::from_bytes_be_slice(&x.to_be_bytes::<{ U256::BYTES }>()[0..U128_BYTES_SIZE]);

        // Insert values in memory
        let base = kakarot_serde
            .runner
            .vm
            .gen_arg(&vec![MaybeRelocatable::Int(low), MaybeRelocatable::Int(high)])
            .unwrap()
            .get_relocatable()
            .unwrap();

        // Serialize the Uint256 struct using the new memory segment.
        let result = kakarot_serde.serialize_uint256(base).expect("failed to serialize pointers");

        // Assert that the result matches the expected U256 value.
        assert_eq!(result, x);
    }

    #[test]
    fn test_serialize_uint256_not_int_high() {
        // Setup the KakarotSerde instance
        let mut kakarot_serde = setup_kakarot_serde();

        // U256 to be serialized
        let x = U256::MAX;

        // Setup with the high and low parts of the U256
        let low =
            Felt252::from_bytes_be_slice(&x.to_be_bytes::<{ U256::BYTES }>()[U128_BYTES_SIZE..]);
        // High is not an Int to trigger the error
        let high = Relocatable { segment_index: 10, offset: 11 };

        // Insert values in memory
        let base = kakarot_serde
            .runner
            .vm
            .gen_arg(&vec![MaybeRelocatable::Int(low), MaybeRelocatable::RelocatableValue(high)])
            .unwrap()
            .get_relocatable()
            .unwrap();

        // Try to serialize the Uint256 struct using the new memory segment.
        let result = kakarot_serde.serialize_uint256(base);

        // Assert that the result is an error with the expected missing field.
        match result {
            Err(KakarotSerdeError::MissingField { field }) => {
                assert_eq!(field, "high");
            }
            _ => panic!("Expected a missing field error, but got: {:?}", result),
        }
    }

    #[test]
    fn test_serialize_uint256_not_int_low() {
        // Setup the KakarotSerde instance
        let mut kakarot_serde = setup_kakarot_serde();

        // U256 to be serialized
        let x = U256::MAX;

        // Low is not an Int to trigger the error
        let low = Relocatable { segment_index: 10, offset: 11 };
        let high =
            Felt252::from_bytes_be_slice(&x.to_be_bytes::<{ U256::BYTES }>()[0..U128_BYTES_SIZE]);

        // Insert values in memory
        let base = kakarot_serde
            .runner
            .vm
            .gen_arg(&vec![MaybeRelocatable::RelocatableValue(low), MaybeRelocatable::Int(high)])
            .unwrap()
            .get_relocatable()
            .unwrap();

        // Try to serialize the Uint256 struct using the new memory segment.
        let result = kakarot_serde.serialize_uint256(base);

        // Assert that the result is an error with the expected missing field.
        match result {
            Err(KakarotSerdeError::MissingField { field }) => {
                assert_eq!(field, "low");
            }
            _ => panic!("Expected a missing field error, but got: {:?}", result),
        }
    }

    #[test]
    fn test_cairo_type_struct_type() {
        // A dummy scope name for the struct type.
        let scope_name = "starkware.cairo.common.uint256.Uint256";

        // Create a Cairo type for the struct.
        let cairo_type = CairoType::struct_type(scope_name, None);

        // Assert that the Cairo type is a struct with the correct scope name.
        assert_eq!(
            cairo_type,
            CairoType::Struct {
                scope: ScopedName {
                    path: vec![
                        "starkware".to_string(),
                        "cairo".to_string(),
                        "common".to_string(),
                        "uint256".to_string(),
                        "Uint256".to_string()
                    ]
                },
                location: None
            }
        );

        // Test with a dummy location
        let location = Some(Location {
            end_line: 100,
            end_col: 454,
            input_file: InputFile { filename: "test.cairo".to_string() },
            parent_location: None,
            start_line: 34,
            start_col: 234,
        });
        let cairo_type_with_location = CairoType::struct_type(scope_name, location.clone());
        assert_eq!(
            cairo_type_with_location,
            CairoType::Struct {
                scope: ScopedName {
                    path: vec![
                        "starkware".to_string(),
                        "cairo".to_string(),
                        "common".to_string(),
                        "uint256".to_string(),
                        "Uint256".to_string()
                    ]
                },
                location
            }
        );
    }

    #[test]
    fn test_cairo_type_felt() {
        // Create a Cairo type for a Felt.
        let cairo_type = CairoType::felt_type(None);

        // Assert that the Cairo type is a Felt with the correct location.
        assert_eq!(cairo_type, CairoType::Felt { location: None });

        // Test with a dummy location
        let location = Some(Location {
            end_line: 100,
            end_col: 454,
            input_file: InputFile { filename: "test.cairo".to_string() },
            parent_location: None,
            start_line: 34,
            start_col: 234,
        });
        let cairo_type_with_location = CairoType::felt_type(location.clone());
        assert_eq!(cairo_type_with_location, CairoType::Felt { location });
    }

    #[test]
    fn test_cairo_type_pointer() {
        // Create a Cairo type for a Pointer.
        let pointee_type = CairoType::felt_type(None);
        let cairo_type = CairoType::pointer_type(pointee_type.clone(), None);

        // Assert that the Cairo type is a Pointer with the correct pointee type.
        assert_eq!(
            cairo_type,
            CairoType::Pointer { pointee: Box::new(pointee_type), location: None }
        );

        // Test with a dummy location
        let location = Some(Location {
            end_line: 100,
            end_col: 454,
            input_file: InputFile { filename: "test.cairo".to_string() },
            parent_location: None,
            start_line: 34,
            start_col: 234,
        });
        let cairo_type_with_location =
            CairoType::pointer_type(CairoType::felt_type(None), location.clone());
        assert_eq!(
            cairo_type_with_location,
            CairoType::Pointer { pointee: Box::new(CairoType::Felt { location: None }), location }
        );
    }

    #[test]
    fn test_cairo_type_tuple() {
        // Create Cairo types for Tuple members.
        let member1 = TupleItem::new(Some("a".to_string()), CairoType::felt_type(None), None);
        let member2 = TupleItem::new(
            Some("b".to_string()),
            CairoType::pointer_type(CairoType::felt_type(None), None),
            None,
        );

        // Create a Cairo type for a Tuple.
        let cairo_type =
            CairoType::tuple_from_members(vec![member1.clone(), member2.clone()], true, None);

        // Assert that the Cairo type is a Tuple with the correct members and trailing comma flag.
        assert_eq!(
            cairo_type,
            CairoType::Tuple {
                members: vec![member1, member2],
                has_trailing_comma: true,
                location: None
            }
        );

        // Test with a dummy location
        let location = Some(Location {
            end_line: 100,
            end_col: 454,
            input_file: InputFile { filename: "test.cairo".to_string() },
            parent_location: None,
            start_line: 34,
            start_col: 234,
        });
        let cairo_type_with_location = CairoType::tuple_from_members(
            vec![TupleItem::new(None, CairoType::felt_type(None), None)],
            false,
            location.clone(),
        );
        assert_eq!(
            cairo_type_with_location,
            CairoType::Tuple {
                members: vec![TupleItem::new(None, CairoType::felt_type(None), None)],
                has_trailing_comma: false,
                location
            }
        );
    }
}
