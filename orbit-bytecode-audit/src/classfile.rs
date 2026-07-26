//! Facade over `ristretto_classfile`.
//!
//! No third-party parser type leaves this module. The rest of the analyzer
//! consumes a deliberately small, owned IR with raw class-file versions,
//! stable instruction IDs, and reconstructed original offsets.

use std::collections::BTreeMap;

use ristretto_classfile::attributes::{AnnotationElement, Attribute, Instruction};
use ristretto_classfile::{
    ClassFile, Constant, ConstantPool, FieldAccessFlags, MethodAccessFlags, ReferenceKind,
};

use crate::model::{ClassDefinitionId, InstructionReference, MemberKind, MemberReference};

const MAX_KNOWN_CLASS_MAJOR: u16 = 69; // Java 25

#[derive(Debug, Clone)]
pub(crate) struct ParsedClass {
    pub definition_id: Option<ClassDefinitionId>,
    pub future_version_best_effort: bool,
    pub name: String,
    pub super_name: Option<String>,
    pub interfaces: Vec<String>,
    pub annotations: Vec<ParsedAnnotation>,
    pub fields: Vec<ParsedField>,
    pub methods: Vec<ParsedMethod>,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedField {
    pub name: String,
    pub descriptor: String,
    pub is_static: bool,
    pub is_private_or_protected: bool,
    pub annotations: Vec<ParsedAnnotation>,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedMethod {
    pub name: String,
    pub descriptor: String,
    pub is_static: bool,
    pub is_public: bool,
    pub is_synthetic: bool,
    pub annotations: Vec<ParsedAnnotation>,
    pub max_locals: Option<u16>,
    pub instructions: Vec<ParsedInstruction>,
}

impl ParsedMethod {
    pub(crate) fn reference(&self, owner: &str) -> MemberReference {
        MemberReference {
            owner: owner.to_string(),
            name: self.name.clone(),
            descriptor: self.descriptor.clone(),
            kind: MemberKind::Method,
            is_static: Some(self.is_static),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedAnnotation {
    pub descriptor: String,
    pub values: BTreeMap<String, AnnotationValue>,
}

impl ParsedAnnotation {
    pub(crate) fn value(&self, name: &str) -> Option<&AnnotationValue> {
        self.values.get(name)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum AnnotationValue {
    String(String),
    Integer(i64),
    Float(String),
    Boolean(bool),
    Class(String),
    Enum { descriptor: String, value: String },
    Annotation(Box<ParsedAnnotation>),
    Array(Vec<AnnotationValue>),
    Unknown(String),
}

impl AnnotationValue {
    pub(crate) fn strings(&self) -> Vec<String> {
        match self {
            Self::String(value) | Self::Class(value) => vec![value.clone()],
            Self::Array(values) => values.iter().flat_map(Self::strings).collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn integer(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn boolean(&self) -> Option<bool> {
        match self {
            Self::Boolean(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn annotations(&self) -> Vec<&ParsedAnnotation> {
        match self {
            Self::Annotation(annotation) => vec![annotation],
            Self::Array(values) => values.iter().flat_map(Self::annotations).collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn render_lossy(&self) -> String {
        match self {
            Self::String(value) => format!("{value:?}"),
            Self::Integer(value) => value.to_string(),
            Self::Float(value) => value.clone(),
            Self::Boolean(value) => value.to_string(),
            Self::Class(value) => format!("class {value}"),
            Self::Enum { descriptor, value } => {
                format!("{descriptor}.{value}")
            }
            Self::Annotation(annotation) => {
                format!("@{}", annotation.descriptor)
            }
            Self::Array(values) => format!(
                "[{}]",
                values
                    .iter()
                    .map(Self::render_lossy)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Unknown(value) => format!("unknown({value})"),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedInstruction {
    pub reference: InstructionReference,
    pub kind: InstructionKind,
}

#[derive(Debug, Clone)]
pub(crate) enum InstructionKind {
    MethodCall(MemberReference),
    FieldRead(MemberReference),
    FieldWrite(MemberReference),
    Type(String),
    StringConstant(String),
    IntegerConstant(i64),
    DecimalConstant(String),
    NullConstant,
    InvokeDynamic {
        name: String,
        descriptor: String,
        implementation: Option<MemberReference>,
    },
    Return,
    Jump,
    Load(u16),
    Store(u16),
    Other,
}

pub(crate) fn parse(bytes: &[u8], max_annotation_depth: usize) -> Result<ParsedClass, String> {
    if bytes.len() < 8 || bytes[..4] != [0xCA, 0xFE, 0xBA, 0xBE] {
        return Err("invalid ClassFile magic or truncated header".to_string());
    }
    let major = u16::from_be_bytes([bytes[6], bytes[7]]);
    let future_version_best_effort = major > MAX_KNOWN_CLASS_MAJOR;
    let patched;
    let parser_bytes = if future_version_best_effort {
        patched = {
            let mut copy = bytes.to_vec();
            copy[6..8].copy_from_slice(&MAX_KNOWN_CLASS_MAJOR.to_be_bytes());
            copy
        };
        patched.as_slice()
    } else {
        bytes
    };
    let class_file = ClassFile::from_bytes(parser_bytes).map_err(|error| error.to_string())?;
    convert(class_file, future_version_best_effort, max_annotation_depth)
}

fn convert(
    class_file: ClassFile<'static>,
    future_version_best_effort: bool,
    max_annotation_depth: usize,
) -> Result<ParsedClass, String> {
    let pool = &class_file.constant_pool;
    let name = class_file
        .class_name()
        .map(java_string)
        .map_err(|error| error.to_string())?;
    let super_name = (class_file.super_class != 0)
        .then(|| pool.try_get_class(class_file.super_class))
        .transpose()
        .map_err(|error| error.to_string())?
        .map(java_string);
    let interfaces = class_file
        .interfaces
        .iter()
        .map(|index| pool.try_get_class(*index).map(java_string))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let bootstrap_methods = class_file
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            Attribute::BootstrapMethods { methods, .. } => Some(methods.as_slice()),
            _ => None,
        })
        .unwrap_or_default();
    let annotations = parse_annotations(pool, &class_file.attributes, 0, max_annotation_depth)?;
    let mut fields = Vec::with_capacity(class_file.fields.len());
    for field in &class_file.fields {
        fields.push(ParsedField {
            name: pool
                .try_get_utf8(field.name_index)
                .map(java_string)
                .map_err(|error| error.to_string())?,
            descriptor: pool
                .try_get_utf8(field.descriptor_index)
                .map(java_string)
                .map_err(|error| error.to_string())?,
            is_static: field.access_flags.contains(FieldAccessFlags::STATIC),
            is_private_or_protected: field
                .access_flags
                .intersects(FieldAccessFlags::PRIVATE | FieldAccessFlags::PROTECTED),
            annotations: parse_annotations(pool, &field.attributes, 0, max_annotation_depth)?,
        });
    }
    let mut methods = Vec::with_capacity(class_file.methods.len());
    for method in &class_file.methods {
        let mut max_locals = None;
        let mut instructions = Vec::new();
        for attribute in &method.attributes {
            if let Attribute::Code {
                max_locals: locals,
                code,
                ..
            } = attribute
            {
                max_locals = Some(*locals);
                instructions = parse_instructions(pool, bootstrap_methods, code);
                break;
            }
        }
        methods.push(ParsedMethod {
            name: pool
                .try_get_utf8(method.name_index)
                .map(java_string)
                .map_err(|error| error.to_string())?,
            descriptor: pool
                .try_get_utf8(method.descriptor_index)
                .map(java_string)
                .map_err(|error| error.to_string())?,
            is_static: method.access_flags.contains(MethodAccessFlags::STATIC),
            is_public: method.access_flags.contains(MethodAccessFlags::PUBLIC),
            is_synthetic: method.access_flags.contains(MethodAccessFlags::SYNTHETIC),
            annotations: parse_annotations(pool, &method.attributes, 0, max_annotation_depth)?,
            max_locals,
            instructions,
        });
    }
    Ok(ParsedClass {
        definition_id: None,
        future_version_best_effort,
        name,
        super_name,
        interfaces,
        annotations,
        fields,
        methods,
    })
}

fn parse_annotations(
    pool: &ConstantPool<'_>,
    attributes: &[Attribute],
    depth: usize,
    max_depth: usize,
) -> Result<Vec<ParsedAnnotation>, String> {
    if depth > max_depth {
        return Err("annotation nesting exceeds parser facade limit".to_string());
    }
    let mut parsed = Vec::new();
    for annotation in attributes.iter().flat_map(|attribute| match attribute {
        Attribute::RuntimeVisibleAnnotations { annotations, .. }
        | Attribute::RuntimeInvisibleAnnotations { annotations, .. } => annotations.as_slice(),
        _ => &[],
    }) {
        parsed.push(convert_annotation(pool, annotation, depth + 1, max_depth)?);
    }
    Ok(parsed)
}

fn convert_annotation(
    pool: &ConstantPool<'_>,
    annotation: &ristretto_classfile::attributes::Annotation,
    depth: usize,
    max_depth: usize,
) -> Result<ParsedAnnotation, String> {
    if depth > max_depth {
        return Err("annotation nesting exceeds parser facade limit".to_string());
    }
    let descriptor = pool
        .try_get_utf8(annotation.type_index)
        .map(java_string)
        .map_err(|error| error.to_string())?;
    let mut values = BTreeMap::new();
    for pair in &annotation.elements {
        let name = pool
            .try_get_utf8(pair.name_index)
            .map(java_string)
            .map_err(|error| error.to_string())?;
        values.insert(
            name,
            convert_annotation_value(pool, &pair.value, depth + 1, max_depth)?,
        );
    }
    Ok(ParsedAnnotation { descriptor, values })
}

fn convert_annotation_value(
    pool: &ConstantPool<'_>,
    value: &AnnotationElement,
    depth: usize,
    max_depth: usize,
) -> Result<AnnotationValue, String> {
    if depth > max_depth {
        return Ok(AnnotationValue::Unknown(
            "annotation nesting limit exceeded".to_string(),
        ));
    }
    let integer = |index| match pool.try_get(index) {
        Ok(Constant::Integer(value)) => Ok(i64::from(*value)),
        Ok(Constant::Long(value)) => Ok(*value),
        Ok(other) => Err(format!(
            "expected integer annotation constant, got {other:?}"
        )),
        Err(error) => Err(error.to_string()),
    };
    let float = |index| match pool.try_get(index) {
        Ok(Constant::Float(value)) => Ok(value.to_string()),
        Ok(Constant::Double(value)) => Ok(value.to_string()),
        Ok(other) => Err(format!("expected float annotation constant, got {other:?}")),
        Err(error) => Err(error.to_string()),
    };
    Ok(match value {
        AnnotationElement::Byte { const_value_index }
        | AnnotationElement::Char { const_value_index }
        | AnnotationElement::Int { const_value_index }
        | AnnotationElement::Long { const_value_index }
        | AnnotationElement::Short { const_value_index } => {
            AnnotationValue::Integer(integer(*const_value_index)?)
        }
        AnnotationElement::Boolean { const_value_index } => {
            AnnotationValue::Boolean(integer(*const_value_index)? != 0)
        }
        AnnotationElement::Float { const_value_index }
        | AnnotationElement::Double { const_value_index } => {
            AnnotationValue::Float(float(*const_value_index)?)
        }
        AnnotationElement::String { const_value_index } => AnnotationValue::String(
            pool.try_get_utf8(*const_value_index)
                .map(java_string)
                .map_err(|error| error.to_string())?,
        ),
        AnnotationElement::Enum {
            type_name_index,
            const_name_index,
        } => AnnotationValue::Enum {
            descriptor: pool
                .try_get_utf8(*type_name_index)
                .map(java_string)
                .map_err(|error| error.to_string())?,
            value: pool
                .try_get_utf8(*const_name_index)
                .map(java_string)
                .map_err(|error| error.to_string())?,
        },
        AnnotationElement::Class { class_info_index } => AnnotationValue::Class(
            pool.try_get_utf8(*class_info_index)
                .map(java_string)
                .map_err(|error| error.to_string())?,
        ),
        AnnotationElement::Annotation { annotation } => AnnotationValue::Annotation(Box::new(
            convert_annotation(pool, annotation, depth + 1, max_depth)?,
        )),
        AnnotationElement::Array { values } => AnnotationValue::Array(
            values
                .iter()
                .map(|value| convert_annotation_value(pool, value, depth + 1, max_depth))
                .collect::<Result<Vec<_>, _>>()?,
        ),
    })
}

fn parse_instructions(
    pool: &ConstantPool<'_>,
    bootstrap_methods: &[ristretto_classfile::attributes::BootstrapMethod],
    code: &[Instruction],
) -> Vec<ParsedInstruction> {
    let mut offset = 0_u32;
    code.iter()
        .enumerate()
        .map(|(index, instruction)| {
            let opcode = instruction.code();
            let kind = instruction_kind(pool, bootstrap_methods, instruction);
            let member = match &kind {
                InstructionKind::MethodCall(member)
                | InstructionKind::FieldRead(member)
                | InstructionKind::FieldWrite(member) => Some(member.clone()),
                _ => None,
            };
            let constant = match &kind {
                InstructionKind::StringConstant(value) => Some(value.clone()),
                InstructionKind::IntegerConstant(value) => Some(value.to_string()),
                InstructionKind::DecimalConstant(value) => Some(value.clone()),
                InstructionKind::NullConstant => Some("null".to_string()),
                _ => None,
            };
            let local_slot = match &kind {
                InstructionKind::Load(slot) | InstructionKind::Store(slot) => Some(*slot),
                _ => None,
            };
            let parsed = ParsedInstruction {
                reference: InstructionReference {
                    identity: None,
                    stable_id: u32::try_from(index).unwrap_or(u32::MAX),
                    original_offset: Some(offset),
                    opcode,
                    local_slot,
                    member,
                    constant,
                },
                kind,
            };
            offset = offset.saturating_add(instruction_length(instruction, offset));
            parsed
        })
        .collect()
}

fn instruction_kind(
    pool: &ConstantPool<'_>,
    bootstrap_methods: &[ristretto_classfile::attributes::BootstrapMethod],
    instruction: &Instruction,
) -> InstructionKind {
    match instruction {
        Instruction::Invokevirtual(index) | Instruction::Invokespecial(index) => {
            resolve_member(pool, *index, MemberKind::Method, Some(false))
                .map_or(InstructionKind::Other, InstructionKind::MethodCall)
        }
        Instruction::Invokestatic(index) => {
            resolve_member(pool, *index, MemberKind::Method, Some(true))
                .map_or(InstructionKind::Other, InstructionKind::MethodCall)
        }
        Instruction::Invokeinterface(index, _) => {
            resolve_member(pool, *index, MemberKind::Method, Some(false))
                .map_or(InstructionKind::Other, InstructionKind::MethodCall)
        }
        Instruction::Getfield(index) => {
            resolve_member(pool, *index, MemberKind::Field, Some(false))
                .map_or(InstructionKind::Other, InstructionKind::FieldRead)
        }
        Instruction::Getstatic(index) => {
            resolve_member(pool, *index, MemberKind::Field, Some(true))
                .map_or(InstructionKind::Other, InstructionKind::FieldRead)
        }
        Instruction::Putfield(index) => {
            resolve_member(pool, *index, MemberKind::Field, Some(false))
                .map_or(InstructionKind::Other, InstructionKind::FieldWrite)
        }
        Instruction::Putstatic(index) => {
            resolve_member(pool, *index, MemberKind::Field, Some(true))
                .map_or(InstructionKind::Other, InstructionKind::FieldWrite)
        }
        Instruction::New(index)
        | Instruction::Anewarray(index)
        | Instruction::Checkcast(index)
        | Instruction::Instanceof(index) => pool
            .try_get_class(*index)
            .ok()
            .map(java_string)
            .map_or(InstructionKind::Other, InstructionKind::Type),
        Instruction::Ldc(index) => constant_kind(pool, u16::from(*index)),
        Instruction::Ldc_w(index) | Instruction::Ldc2_w(index) => constant_kind(pool, *index),
        Instruction::Bipush(value) => InstructionKind::IntegerConstant(i64::from(*value)),
        Instruction::Sipush(value) => InstructionKind::IntegerConstant(i64::from(*value)),
        Instruction::Iconst_m1 => InstructionKind::IntegerConstant(-1),
        Instruction::Iconst_0 => InstructionKind::IntegerConstant(0),
        Instruction::Iconst_1 => InstructionKind::IntegerConstant(1),
        Instruction::Iconst_2 => InstructionKind::IntegerConstant(2),
        Instruction::Iconst_3 => InstructionKind::IntegerConstant(3),
        Instruction::Iconst_4 => InstructionKind::IntegerConstant(4),
        Instruction::Iconst_5 => InstructionKind::IntegerConstant(5),
        Instruction::Lconst_0 => InstructionKind::IntegerConstant(0),
        Instruction::Lconst_1 => InstructionKind::IntegerConstant(1),
        Instruction::Fconst_0 => InstructionKind::DecimalConstant("0".to_string()),
        Instruction::Fconst_1 => InstructionKind::DecimalConstant("1".to_string()),
        Instruction::Fconst_2 => InstructionKind::DecimalConstant("2".to_string()),
        Instruction::Dconst_0 => InstructionKind::DecimalConstant("0".to_string()),
        Instruction::Dconst_1 => InstructionKind::DecimalConstant("1".to_string()),
        Instruction::Aconst_null => InstructionKind::NullConstant,
        Instruction::Invokedynamic(index) => resolve_invokedynamic(pool, bootstrap_methods, *index),
        Instruction::Ireturn
        | Instruction::Lreturn
        | Instruction::Freturn
        | Instruction::Dreturn
        | Instruction::Areturn
        | Instruction::Return => InstructionKind::Return,
        Instruction::Ifeq(_)
        | Instruction::Ifne(_)
        | Instruction::Iflt(_)
        | Instruction::Ifge(_)
        | Instruction::Ifgt(_)
        | Instruction::Ifle(_)
        | Instruction::If_icmpeq(_)
        | Instruction::If_icmpne(_)
        | Instruction::If_icmplt(_)
        | Instruction::If_icmpge(_)
        | Instruction::If_icmpgt(_)
        | Instruction::If_icmple(_)
        | Instruction::If_acmpeq(_)
        | Instruction::If_acmpne(_)
        | Instruction::Goto(_)
        | Instruction::Jsr(_)
        | Instruction::Tableswitch(_)
        | Instruction::Lookupswitch(_)
        | Instruction::Ifnull(_)
        | Instruction::Ifnonnull(_)
        | Instruction::Goto_w(_)
        | Instruction::Jsr_w(_) => InstructionKind::Jump,
        _ => local_instruction(instruction).unwrap_or(InstructionKind::Other),
    }
}

fn constant_kind(pool: &ConstantPool<'_>, index: u16) -> InstructionKind {
    match pool.try_get(index) {
        Ok(Constant::String(utf8)) => pool
            .try_get_utf8(*utf8)
            .ok()
            .map(java_string)
            .map_or(InstructionKind::Other, InstructionKind::StringConstant),
        Ok(Constant::Utf8(value)) => InstructionKind::StringConstant(java_string(value)),
        Ok(Constant::Integer(value)) => InstructionKind::IntegerConstant(i64::from(*value)),
        Ok(Constant::Long(value)) => InstructionKind::IntegerConstant(*value),
        Ok(Constant::Float(value)) => InstructionKind::DecimalConstant(value.to_string()),
        Ok(Constant::Double(value)) => InstructionKind::DecimalConstant(value.to_string()),
        Ok(Constant::Class(name_index)) => pool
            .try_get_utf8(*name_index)
            .ok()
            .map(java_string)
            .map_or(InstructionKind::Other, InstructionKind::Type),
        _ => InstructionKind::Other,
    }
}

fn resolve_invokedynamic(
    pool: &ConstantPool<'_>,
    bootstrap_methods: &[ristretto_classfile::attributes::BootstrapMethod],
    index: u16,
) -> InstructionKind {
    let Ok(Constant::InvokeDynamic {
        bootstrap_method_attr_index,
        name_and_type_index,
    }) = pool.try_get(index)
    else {
        return InstructionKind::Other;
    };
    let Some((name, descriptor)) = resolve_name_and_type(pool, *name_and_type_index) else {
        return InstructionKind::Other;
    };
    let implementation = bootstrap_methods
        .get(usize::from(*bootstrap_method_attr_index))
        .into_iter()
        .flat_map(|bootstrap| &bootstrap.arguments)
        .find_map(|argument| resolve_method_handle(pool, *argument));
    InstructionKind::InvokeDynamic {
        name,
        descriptor,
        implementation,
    }
}

fn resolve_method_handle(pool: &ConstantPool<'_>, index: u16) -> Option<MemberReference> {
    let Constant::MethodHandle {
        reference_kind,
        reference_index,
    } = pool.try_get(index).ok()?
    else {
        return None;
    };
    let is_static = matches!(reference_kind, ReferenceKind::InvokeStatic);
    resolve_member(pool, *reference_index, MemberKind::Method, Some(is_static))
}

fn resolve_member(
    pool: &ConstantPool<'_>,
    index: u16,
    kind: MemberKind,
    is_static: Option<bool>,
) -> Option<MemberReference> {
    let (class_index, name_and_type_index) = match (kind, pool.try_get(index).ok()?) {
        (
            MemberKind::Field,
            Constant::FieldRef {
                class_index,
                name_and_type_index,
            },
        )
        | (
            MemberKind::Method,
            Constant::MethodRef {
                class_index,
                name_and_type_index,
            },
        )
        | (
            MemberKind::Method,
            Constant::InterfaceMethodRef {
                class_index,
                name_and_type_index,
            },
        ) => (*class_index, *name_and_type_index),
        _ => return None,
    };
    let owner = pool.try_get_class(class_index).ok().map(java_string)?;
    let (name, descriptor) = resolve_name_and_type(pool, name_and_type_index)?;
    Some(MemberReference {
        owner,
        name,
        descriptor,
        kind,
        is_static,
    })
}

fn resolve_name_and_type(pool: &ConstantPool<'_>, index: u16) -> Option<(String, String)> {
    let (name_index, descriptor_index) = pool.try_get_name_and_type(index).ok()?;
    Some((
        pool.try_get_utf8(*name_index).ok().map(java_string)?,
        pool.try_get_utf8(*descriptor_index).ok().map(java_string)?,
    ))
}

fn local_instruction(instruction: &Instruction) -> Option<InstructionKind> {
    use Instruction::*;
    let load = match instruction {
        Iload(index) | Lload(index) | Fload(index) | Dload(index) | Aload(index) => {
            Some(u16::from(*index))
        }
        Iload_0 | Lload_0 | Fload_0 | Dload_0 | Aload_0 => Some(0),
        Iload_1 | Lload_1 | Fload_1 | Dload_1 | Aload_1 => Some(1),
        Iload_2 | Lload_2 | Fload_2 | Dload_2 | Aload_2 => Some(2),
        Iload_3 | Lload_3 | Fload_3 | Dload_3 | Aload_3 => Some(3),
        Iload_w(index) | Lload_w(index) | Fload_w(index) | Dload_w(index) | Aload_w(index) => {
            Some(*index)
        }
        _ => None,
    };
    if let Some(index) = load {
        return Some(InstructionKind::Load(index));
    }
    let store = match instruction {
        Istore(index) | Lstore(index) | Fstore(index) | Dstore(index) | Astore(index) => {
            Some(u16::from(*index))
        }
        Istore_0 | Lstore_0 | Fstore_0 | Dstore_0 | Astore_0 => Some(0),
        Istore_1 | Lstore_1 | Fstore_1 | Dstore_1 | Astore_1 => Some(1),
        Istore_2 | Lstore_2 | Fstore_2 | Dstore_2 | Astore_2 => Some(2),
        Istore_3 | Lstore_3 | Fstore_3 | Dstore_3 | Astore_3 => Some(3),
        Istore_w(index) | Lstore_w(index) | Fstore_w(index) | Dstore_w(index) | Astore_w(index) => {
            Some(*index)
        }
        _ => None,
    };
    store.map(InstructionKind::Store)
}

fn instruction_length(instruction: &Instruction, offset: u32) -> u32 {
    use Instruction::{
        Aload_w, Astore_w, Dload_w, Dstore_w, Fload_w, Fstore_w, Iinc_w, Iload_w, Istore_w,
        Lload_w, Lookupswitch, Lstore_w, Tableswitch,
    };
    match instruction {
        Tableswitch(table) => {
            let padding = (4 - ((offset + 1) % 4)) % 4;
            1 + padding + 12 + 4 * u32::try_from(table.offsets.len()).unwrap_or(u32::MAX)
        }
        Lookupswitch(lookup) => {
            let padding = (4 - ((offset + 1) % 4)) % 4;
            1 + padding + 8 + 8 * u32::try_from(lookup.pairs.len()).unwrap_or(u32::MAX)
        }
        Iload_w(_) | Lload_w(_) | Fload_w(_) | Dload_w(_) | Aload_w(_) | Istore_w(_)
        | Lstore_w(_) | Fstore_w(_) | Dstore_w(_) | Astore_w(_) => 4,
        Iinc_w(_, _) => 6,
        _ => match instruction.code() {
            16 | 18 | 21..=25 | 54..=58 | 169 | 188 => 2,
            17 | 19 | 20 | 132 | 153..=168 | 178..=184 | 187 | 189 | 192 | 193 | 198 | 199 => 3,
            185 | 186 | 200 | 201 => 5,
            197 => 4,
            _ => 1,
        },
    }
}

fn java_string(value: &ristretto_classfile::JavaStr) -> String {
    value.to_string()
}
