use std::io::Cursor;

use solana_ed25519_program::new_ed25519_instruction_with_signature;
use solana_keypair::{Keypair, read_keypair};
use solana_signature::Signature;
use solana_signer::Signer;

const RFC8032_SEED_HEX: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const RFC8032_PUBLIC_KEY_HEX: &str =
    "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
const RFC8032_EMPTY_MESSAGE_SIGNATURE_HEX: &str = concat!(
    "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155",
    "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
);

fn decode_hex<const N: usize>(value: &str) -> [u8; N] {
    assert_eq!(value.len(), N * 2);
    let mut decoded = [0_u8; N];
    for (index, byte) in decoded.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).expect("valid hex");
    }
    decoded
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().expect("u16 field"))
}

#[test]
fn solana_keypair_and_signature_match_the_rfc8032_wire_vector() {
    let seed = decode_hex::<32>(RFC8032_SEED_HEX);
    let expected_public_key = decode_hex::<32>(RFC8032_PUBLIC_KEY_HEX);
    let expected_signature = decode_hex::<64>(RFC8032_EMPTY_MESSAGE_SIGNATURE_HEX);

    let keypair = Keypair::new_from_array(seed);
    let signature = keypair.sign_message(b"");

    assert_eq!(keypair.pubkey().to_bytes(), expected_public_key);
    assert_eq!(signature.as_ref(), expected_signature);
    assert!(signature.verify(expected_public_key.as_ref(), b""));
    assert!(!signature.verify(expected_public_key.as_ref(), b"changed"));

    let encoded_keypair = format!(
        "[{}]",
        keypair
            .to_bytes()
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    let decoded_keypair = read_keypair(&mut Cursor::new(encoded_keypair)).expect("keypair JSON");
    assert_eq!(decoded_keypair.to_bytes(), keypair.to_bytes());
}

#[test]
fn ed25519_instruction_builder_preserves_solana_wire_layout() {
    let public_key = decode_hex::<32>(RFC8032_PUBLIC_KEY_HEX);
    let signature_bytes = decode_hex::<64>(RFC8032_EMPTY_MESSAGE_SIGNATURE_HEX);
    let instruction = new_ed25519_instruction_with_signature(b"", &signature_bytes, &public_key);

    assert_eq!(
        instruction.program_id.to_string(),
        "Ed25519SigVerify111111111111111111111111111"
    );
    assert!(instruction.accounts.is_empty());
    assert_eq!(instruction.data.len(), 112);
    assert_eq!(&instruction.data[..2], &[1, 0]);
    assert_eq!(read_u16(&instruction.data, 2), 48);
    assert_eq!(read_u16(&instruction.data, 4), u16::MAX);
    assert_eq!(read_u16(&instruction.data, 6), 16);
    assert_eq!(read_u16(&instruction.data, 8), u16::MAX);
    assert_eq!(read_u16(&instruction.data, 10), 112);
    assert_eq!(read_u16(&instruction.data, 12), 0);
    assert_eq!(read_u16(&instruction.data, 14), u16::MAX);
    assert_eq!(&instruction.data[16..48], public_key.as_ref());
    assert_eq!(&instruction.data[48..112], signature_bytes.as_ref());

    let parsed = Signature::try_from(signature_bytes.as_ref()).expect("signature bytes");
    assert_eq!(parsed, Signature::from(signature_bytes));
}
