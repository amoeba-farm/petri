//! Instructions for the [ed25519 native program][np].
//!
//! [np]: https://docs.solanalabs.com/runtime/programs#ed25519-program

use {
    bytemuck::bytes_of,
    bytemuck_derive::{Pod, Zeroable},
    ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey},
    solana_instruction::Instruction,
    solana_precompile_error::PrecompileError,
};

pub const PUBKEY_SERIALIZED_SIZE: usize = 32;
pub const SIGNATURE_SERIALIZED_SIZE: usize = 64;
pub const SIGNATURE_OFFSETS_SERIALIZED_SIZE: usize = 14;
// bytemuck requires structures to be aligned
pub const SIGNATURE_OFFSETS_START: usize = 2;
pub const DATA_START: usize = SIGNATURE_OFFSETS_SERIALIZED_SIZE + SIGNATURE_OFFSETS_START;

#[derive(Default, Debug, Copy, Clone, Zeroable, Pod, Eq, PartialEq)]
#[repr(C)]
pub struct Ed25519SignatureOffsets {
    pub signature_offset: u16, // offset to ed25519 signature of 64 bytes
    pub signature_instruction_index: u16, // instruction index to find signature
    pub public_key_offset: u16, // offset to public key of 32 bytes
    pub public_key_instruction_index: u16, // instruction index to find public key
    pub message_data_offset: u16, // offset to start of message data
    pub message_data_size: u16, // size of message data
    pub message_instruction_index: u16, // index of instruction data to get message data
}

/// Encode just the signature offsets in a single ed25519 instruction.
///
/// This is a convenience function for rare cases where we wish to verify multiple messages in
/// the same instruction. The verification data can be stored in a separate instruction specified
/// by the `*_instruction_index` fields of `offsets`, or in this instruction by extending the data
/// buffer.
///
/// Note: If the signer for these messages are the same, it is cheaper to concatenate the messages
/// and have the signer sign the single buffer and use [`new_ed25519_instruction_with_signature`].
pub fn offsets_to_ed25519_instruction(offsets: &[Ed25519SignatureOffsets]) -> Instruction {
    let mut instruction_data = Vec::with_capacity(
        SIGNATURE_OFFSETS_START
            .saturating_add(SIGNATURE_OFFSETS_SERIALIZED_SIZE.saturating_mul(offsets.len())),
    );

    let num_signatures = offsets.len() as u16;
    instruction_data.extend_from_slice(&num_signatures.to_le_bytes());

    for offsets in offsets {
        instruction_data.extend_from_slice(bytes_of(offsets));
    }

    Instruction {
        program_id: solana_sdk_ids::ed25519_program::id(),
        accounts: vec![],
        data: instruction_data,
    }
}

#[deprecated(since = "2.2.3", note = "Use new_ed25519_instruction_with_signature")]
pub fn new_ed25519_instruction(keypair: &ed25519_dalek::SigningKey, message: &[u8]) -> Instruction {
    let signature = keypair.sign(message).to_bytes();
    let pubkey = keypair.verifying_key().to_bytes();
    new_ed25519_instruction_with_signature(message, &signature, &pubkey)
}

pub fn new_ed25519_instruction_with_signature(
    message: &[u8],
    signature: &[u8; SIGNATURE_SERIALIZED_SIZE],
    pubkey: &[u8; PUBKEY_SERIALIZED_SIZE],
) -> Instruction {
    let mut instruction_data = Vec::with_capacity(
        DATA_START
            .saturating_add(SIGNATURE_SERIALIZED_SIZE)
            .saturating_add(PUBKEY_SERIALIZED_SIZE)
            .saturating_add(message.len()),
    );

    let num_signatures: u8 = 1;
    let public_key_offset = DATA_START;
    let signature_offset = public_key_offset.saturating_add(PUBKEY_SERIALIZED_SIZE);
    let message_data_offset = signature_offset.saturating_add(SIGNATURE_SERIALIZED_SIZE);

    // add padding byte so that offset structure is aligned
    instruction_data.extend_from_slice(bytes_of(&[num_signatures, 0]));

    let offsets = Ed25519SignatureOffsets {
        signature_offset: signature_offset as u16,
        signature_instruction_index: u16::MAX,
        public_key_offset: public_key_offset as u16,
        public_key_instruction_index: u16::MAX,
        message_data_offset: message_data_offset as u16,
        message_data_size: message.len() as u16,
        message_instruction_index: u16::MAX,
    };

    instruction_data.extend_from_slice(bytes_of(&offsets));

    debug_assert_eq!(instruction_data.len(), public_key_offset);

    instruction_data.extend_from_slice(pubkey);

    debug_assert_eq!(instruction_data.len(), signature_offset);

    instruction_data.extend_from_slice(signature);

    debug_assert_eq!(instruction_data.len(), message_data_offset);

    instruction_data.extend_from_slice(message);

    Instruction {
        program_id: solana_sdk_ids::ed25519_program::id(),
        accounts: vec![],
        data: instruction_data,
    }
}

#[deprecated(
    since = "2.2.3",
    note = "Use agave_precompiles::ed25519::verify instead"
)]
#[allow(deprecated)]
pub fn verify(
    data: &[u8],
    instruction_datas: &[&[u8]],
    feature_set: &solana_feature_set::FeatureSet,
) -> Result<(), PrecompileError> {
    if data.len() < SIGNATURE_OFFSETS_START {
        return Err(PrecompileError::InvalidInstructionDataSize);
    }
    let num_signatures = data[0] as usize;
    if num_signatures == 0 && data.len() > SIGNATURE_OFFSETS_START {
        return Err(PrecompileError::InvalidInstructionDataSize);
    }
    let expected_data_size = num_signatures
        .saturating_mul(SIGNATURE_OFFSETS_SERIALIZED_SIZE)
        .saturating_add(SIGNATURE_OFFSETS_START);
    // We do not check or use the byte at data[1]
    if data.len() < expected_data_size {
        return Err(PrecompileError::InvalidInstructionDataSize);
    }
    for i in 0..num_signatures {
        let start = i
            .saturating_mul(SIGNATURE_OFFSETS_SERIALIZED_SIZE)
            .saturating_add(SIGNATURE_OFFSETS_START);
        let end = start.saturating_add(SIGNATURE_OFFSETS_SERIALIZED_SIZE);

        // bytemuck wants structures aligned
        let offsets: &Ed25519SignatureOffsets = bytemuck::try_from_bytes(&data[start..end])
            .map_err(|_| PrecompileError::InvalidDataOffsets)?;

        // Parse out signature
        let signature = get_data_slice(
            data,
            instruction_datas,
            offsets.signature_instruction_index,
            offsets.signature_offset,
            SIGNATURE_SERIALIZED_SIZE,
        )?;

        let signature =
            Signature::from_slice(signature).map_err(|_| PrecompileError::InvalidSignature)?;

        // Parse out pubkey
        let pubkey = get_data_slice(
            data,
            instruction_datas,
            offsets.public_key_instruction_index,
            offsets.public_key_offset,
            PUBKEY_SERIALIZED_SIZE,
        )?;

        let publickey = VerifyingKey::from_bytes(
            pubkey
                .try_into()
                .map_err(|_| PrecompileError::InvalidPublicKey)?,
        )
        .map_err(|_| PrecompileError::InvalidPublicKey)?;

        // Parse out message
        let message = get_data_slice(
            data,
            instruction_datas,
            offsets.message_instruction_index,
            offsets.message_data_offset,
            offsets.message_data_size as usize,
        )?;

        if feature_set.is_active(&solana_feature_set::ed25519_precompile_verify_strict::id()) {
            publickey
                .verify_strict(message, &signature)
                .map_err(|_| PrecompileError::InvalidSignature)?;
        } else {
            publickey
                .verify(message, &signature)
                .map_err(|_| PrecompileError::InvalidSignature)?;
        }
    }
    Ok(())
}

fn get_data_slice<'a>(
    data: &'a [u8],
    instruction_datas: &'a [&[u8]],
    instruction_index: u16,
    offset_start: u16,
    size: usize,
) -> Result<&'a [u8], PrecompileError> {
    let instruction = if instruction_index == u16::MAX {
        data
    } else {
        let signature_index = instruction_index as usize;
        if signature_index >= instruction_datas.len() {
            return Err(PrecompileError::InvalidDataOffsets);
        }
        instruction_datas[signature_index]
    };

    let start = offset_start as usize;
    let end = start.saturating_add(size);
    if end > instruction.len() {
        return Err(PrecompileError::InvalidDataOffsets);
    }

    Ok(&instruction[start..end])
}
