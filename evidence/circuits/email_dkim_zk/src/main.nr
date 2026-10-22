// email_dkim_zk: a DKIM-signed quote, proven without disclosing the message.
//
// Proves, for a message nobody but the prover sees:
//
// - an RSA-2048 PKCS#1 v1.5 SHA-256 signature by the public key (`modulus`,
//   `redc`) over `header`, the DKIM header input (the signed header fields,
//   relaxed-canonicalized, then the DKIM-Signature field with `b=` empty);
// - that DKIM-Signature field is the last field of the input (an earlier
//   signed copy of another signature is never read), uses relaxed header
//   canonicalization (`c=relaxed/...`), has exactly one `d=`, `s=` and
//   `bh=`, at most one `t=`, and no `l=` (no tag starting with `l`): the
//   signature covers the whole body;
// - `bh=` is the SHA-256 of `body`, the canonical body;
// - the signed header input has exactly one `content-type:` field and at
//   most one `content-transfer-encoding:` field, whose values are disclosed
//   (the verifier applies the email method's MIME rules to them), and
//   exactly one `from:` field, whose address is or is not within `d=`
//   (`from_aligned`, computed here);
// - for each disclosed label, exactly one line outside quoted (`>`) lines
//   starts with it (after any leading whitespace `str::trim` could skip),
//   and that line is: spaces or tabs, the label, spaces or tabs, the
//   disclosed value, spaces or tabs, CRLF;
// - the body contains none of the forwarded-message markers.
//
// Unsigned header fields never enter the proof, so nothing unsigned is ever
// read or disclosed. Bytes past the header's or the body's length are never
// read as signed: every disclosed byte and every structural check is
// bounded by the length, and garbage there can only make a proof fail.
//
// Size bounds are this circuit's, not policy: a longer header input, body,
// labelled line, domain or MIME value cannot be proven with it.
//
// Cost notes: byte tests go through constant tables and booleans are 0/1
// field elements (an equality chain on a byte costs several gates; a table
// lookup about one).

use base64::BASE64_DECODER;
use bignum::{params::BigNumParams, RuntimeBigNum};
use rsa::{rsa::verify_sha256_pkcs1v15, types::RBN2048};
use sha256::sha256_var;

global MAX_HEADER: u32 = 1024;
global MAX_BODY: u32 = 4096;
global MAX_LINES: u32 = 256;
global SLOTS: u32 = 10;
global KEY_LIMBS: u32 = 18;
global RSA_EXPONENT: u32 = 65537;

// Packed public strings: 31 bytes per field element, big-endian, zero after
// the length.
global LABEL_FIELDS: u32 = 4; // 124 bytes (manifest labels are at most 100)
global VALUE_FIELDS: u32 = 4; // 124 bytes
global DOMAIN_FIELDS: u32 = 9; // 279 bytes (a DNS name is at most 253)
global SELECTOR_FIELDS: u32 = 4;
global CT_FIELDS: u32 = 4;
global CTE_FIELDS: u32 = 1;

global PACK: u32 = 31;
global LABEL_BYTES: u32 = LABEL_FIELDS * PACK;
global VALUE_BYTES: u32 = VALUE_FIELDS * PACK;
global DOMAIN_BYTES: u32 = DOMAIN_FIELDS * PACK;
global SELECTOR_BYTES: u32 = SELECTOR_FIELDS * PACK;
// A labelled line, from its start (indentation included) to its CRLF.
global LINE_BYTES: u32 = 192;
// The From field's value, to its CRLF.
global FROM_BYTES: u32 = 256;

// The header with a CRLF in front (so the first field also follows one),
// padded far enough for every window read.
global HP: u32 = MAX_HEADER + 2 + 320;
// The body, padded for the line windows.
global BP: u32 = MAX_BODY + LINE_BYTES + 2;

global CR: u8 = 13;
global LF: u8 = 10;
global SP: u8 = 32;
global SEMI: u8 = 59;
global EQ: u8 = 61;
global GT: u8 = 62;
global DOT: u8 = 46;

// The fingerprint base. Any fixed value works: a real occurrence always
// matches its fingerprint, so counts never miss one; a collision could only
// make an honest proof fail. (Fingerprints are never used to show two
// strings equal: disclosed bytes are compared byte by byte.)
global R: Field = 0x1d9a8e5c6b4f3a2e1d0c9b8a7f6e5d4c3b2a19087f6e5d4c3b2a1908f7e6d5c;

// Byte classes, 1 or 0.
global WSP: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[32] = 1;
    t[9] = 1;
    t
};
// What `str::trim` may take for whitespace at a line's start, as bytes:
// spaces, tabs, CR, VT, FF and any non-ASCII byte (Unicode spaces are
// non-ASCII). Wider than whitespace on purpose: counting at the first byte
// past these finds every line a label could start.
global LOOSE: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    for i in 0..256 {
        if (i == 32) | (i == 9) | (i == 13) | (i == 11) | (i == 12) | (i >= 128) {
            t[i] = 1;
        }
    }
    t
};
global IS_CR: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[13] = 1;
    t
};
global IS_LF: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[10] = 1;
    t
};
global IS_CRLF_OR_ZERO: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[0] = 1;
    t[10] = 1;
    t[13] = 1;
    t
};
global IS_GT: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[62] = 1;
    t
};
global IS_SEMI: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[59] = 1;
    t
};
global IS_EQ: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[61] = 1;
    t
};
global IS_AT: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[64] = 1;
    t
};
// A tag's first letter: d s t c b l h, as 1 in its own table.
global IS_D: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[100] = 1;
    t
};
global IS_S: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[115] = 1;
    t
};
global IS_T: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[116] = 1;
    t
};
global IS_C: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[99] = 1;
    t
};
global IS_B: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[98] = 1;
    t
};
global IS_L: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[108] = 1;
    t
};
global IS_H: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[104] = 1;
    t
};
// Not a tag value byte: WSP, `;`, CR, LF, 0.
global BAD_TAG_BYTE: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[0] = 1;
    t[9] = 1;
    t[10] = 1;
    t[13] = 1;
    t[32] = 1;
    t[59] = 1;
    t
};
// From's tail: spaces, tabs and `>` are stripped after the domain.
global FROM_STRIP: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    t[9] = 1;
    t[32] = 1;
    t[62] = 1;
    t
};
global LOWER: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    for i in 0..256 {
        t[i] = if (i >= 65) & (i <= 90) { (i + 32) as Field } else { i as Field };
    }
    t
};
global DIGIT: [Field; 256] = {
    let mut t: [Field; 256] = [0; 256];
    for i in 48..58 {
        t[i] = 1;
    }
    t
};

// A fresh witness equal to `x`. A value carried through a loop as a sum
// (`x = x + a*b`, or a select `a*j + (1-a)*x`, which is `x - a*x + a*j`)
// stays a growing linear expression, and multiplying it expands every term:
// quadratic in the loop. Pinning it to one witness costs one gate.
unconstrained fn __copy(x: Field) -> Field {
    x
}

fn pin(x: Field) -> Field {
    // Safety: constrained equal to `x` on the next line.
    let y = unsafe { __copy(x) };
    assert(y == x);
    y
}

fn cls(table: [Field; 256], c: u8) -> Field {
    table[c as u32]
}

fn eqf(a: u32, b: u32) -> Field {
    (a == b) as Field
}

fn bit(x: Field) -> bool {
    x != 0
}

// Prefix fingerprints: p[i] = sum_{j<i} s[j] R^j, pw[i] = R^i.
fn fingerprints<let N: u32>(s: [u8; N]) -> ([Field; N + 1], [Field; N + 1]) {
    let mut p: [Field; N + 1] = [0; N + 1];
    let mut pw: [Field; N + 1] = [0; N + 1];
    let mut acc: Field = 0;
    let mut r: Field = 1;
    for i in 0..N {
        p[i] = acc;
        pw[i] = r;
        acc += (s[i] as Field) * r;
        r *= R;
    }
    p[N] = acc;
    pw[N] = r;
    (p, pw)
}

fn fingerprint_of<let L: u32>(pat: [u8; L]) -> Field {
    let mut acc: Field = 0;
    let mut r: Field = 1;
    for j in 0..L {
        acc += (pat[j] as Field) * r;
        r *= R;
    }
    acc
}

// How many times the constant pattern occurs in the windows starting at
// 0..LIMIT.
fn count_const<let N: u32, let L: u32, let LIMIT: u32>(
    p: [Field; N + 1],
    pw: [Field; N + 1],
    pat: [u8; L],
) -> Field {
    let h = fingerprint_of(pat);
    let mut count: Field = 0;
    for i in 0..LIMIT {
        let z = p[i + L] - p[i] - h * pw[i];
        count += (z == 0) as Field;
    }
    count
}

// The pattern occurs in no window starting at 0..LIMIT: one product, one
// inverse.
fn absent<let N: u32, let L: u32, let LIMIT: u32>(
    p: [Field; N + 1],
    pw: [Field; N + 1],
    pat: [u8; L],
) -> Field {
    let h = fingerprint_of(pat);
    let mut prod: Field = 1;
    for i in 0..LIMIT {
        prod *= p[i + L] - p[i] - h * pw[i];
    }
    prod
}

// Unpacks a public string and checks it is canonical: non-zero bytes up to
// `len`, zeros after.
fn unpack<let F: u32>(fields: [Field; F], len: u32) -> [u8; F * 31] {
    let mut out: [u8; F * 31] = [0; F * 31];
    for f in 0..F {
        let bytes: [u8; 31] = fields[f].to_be_bytes();
        for j in 0..31 {
            out[f * 31 + j] = bytes[j];
        }
    }
    assert(len <= F * 31);
    let mut below: Field = 1; // j < len
    for j in 0..F * 31 {
        below = below * (1 - eqf(j, len));
        let b = out[j] as Field;
        // Past the length: zero. Before it: non-zero.
        assert((1 - below) * b == 0);
        if bit(below) {
            assert(b != 0);
        }
    }
    out
}

fn window<let N: u32, let W: u32>(s: [u8; N], start: u32) -> [u8; W] {
    let mut out: [u8; W] = [0; W];
    for j in 0..W {
        out[j] = s[start + j];
    }
    out
}

fn limbs_u128(f: [Field; KEY_LIMBS]) -> [u128; KEY_LIMBS] {
    let mut out: [u128; KEY_LIMBS] = [0; KEY_LIMBS];
    for i in 0..KEY_LIMBS {
        f[i].assert_max_bit_size::<120>();
        out[i] = f[i] as u128;
    }
    out
}

// A DKIM tag value at `v` (padded-header coordinates) of length `len`:
// non-empty, no WSP, `;`, CR, LF or zero inside, followed by an optional
// space and then `;` or the field's end (`end`), all before `end`. `v` is
// the tag's `=` plus one, or plus two after a space.
fn assert_tag_value<let W: u32>(h: [u8; HP], eq: u32, v: u32, len: u32, end: u32) -> [u8; W] {
    assert(h[eq] == EQ);
    assert((v == eq + 1) | ((v == eq + 2) & (h[eq + 1] == SP)));
    assert(len > 0);
    assert(len <= W);
    let after = v + len;
    assert(after <= end, "a DKIM tag value runs past the header");
    let w: [u8; W] = window(h, v);
    let mut out: [u8; W] = [0; W];
    let mut below: Field = 1;
    for j in 0..W {
        below = below * (1 - eqf(j, len));
        assert(below * cls(BAD_TAG_BYTE, w[j]) == 0);
        out[j] = (below * (w[j] as Field)) as u8;
    }
    let at_end = after == end;
    let semi = h[after] == SEMI;
    let sp_then = (h[after] == SP) & ((after + 1 == end) | (h[after + 1] == SEMI));
    assert(at_end | semi | sp_then, "a DKIM tag value does not end at `;`");
    out
}

fn pack_check<let F: u32>(bytes: [u8; F * 31], fields: [Field; F]) {
    for f in 0..F {
        let mut acc: Field = 0;
        for j in 0..31 {
            acc = acc * 256 + (bytes[f * 31 + j] as Field);
        }
        assert(acc == fields[f]);
    }
}

// A signed header field's value: right after `name_end` up to its CRLF,
// equal to the disclosed bytes, the CRLF before the header's end.
fn assert_field_value<let W: u32>(h: [u8; HP], start: u32, bytes: [u8; W], len: u32, hend: u32) {
    let v: [u8; W + 2] = window(h, start);
    let mut below: Field = 1;
    for j in 0..W {
        below = below * (1 - eqf(j, len));
        assert(below * ((v[j] as Field) - (bytes[j] as Field)) == 0);
        assert(below * cls(IS_CRLF_OR_ZERO, v[j]) == 0);
    }
    assert((h[start + len] == CR) & (h[start + len + 1] == LF));
    assert(start + len + 1 < hend, "a header field runs past the header");
}

fn main(
    header: BoundedVec<u8, MAX_HEADER>,
    body: BoundedVec<u8, MAX_BODY>,
    signature: [Field; KEY_LIMBS],
    // Hints (positions in the padded header: header index + 2).
    dkim_start: u32,
    d_tag: u32,
    d_value: u32,
    s_tag: u32,
    s_value: u32,
    t_tag: u32,
    t_len: u32,
    c_tag: u32,
    bh_tag: u32,
    ct_field: u32,
    cte_field: u32,
    from_field: u32,
    // Positions in the body.
    n_lines: u32,
    line_starts: [u32; MAX_LINES],
    content_starts: [u32; MAX_LINES],
    label_line: [u32; SLOTS],
    value_at: [u32; SLOTS],
    // Public.
    modulus: pub [Field; KEY_LIMBS],
    redc: pub [Field; KEY_LIMBS],
    domain: pub [Field; DOMAIN_FIELDS],
    domain_len: pub u32,
    selector: pub [Field; SELECTOR_FIELDS],
    selector_len: pub u32,
    sig_time: pub u64,
    content_type: pub [Field; CT_FIELDS],
    content_type_len: pub u32,
    cte: pub [Field; CTE_FIELDS],
    cte_len: pub u32,
    from_aligned: pub bool,
    labels: pub [[Field; LABEL_FIELDS]; SLOTS],
    label_lens: pub [u32; SLOTS],
    values: pub [[Field; VALUE_FIELDS]; SLOTS],
    value_lens: pub [u32; SLOTS],
) {
    let hlen = header.len();
    let blen = body.len();
    assert(hlen <= MAX_HEADER);
    assert(blen <= MAX_BODY);

    // 1. The signature over the header input.
    let header_hash = sha256_var(header.storage(), hlen);
    let params: BigNumParams<KEY_LIMBS, 2048> =
        BigNumParams::new(false, limbs_u128(modulus), limbs_u128(redc));
    let sig: RBN2048 = RuntimeBigNum::from_array(params, limbs_u128(signature));
    sig.validate_in_range();
    assert(verify_sha256_pkcs1v15(header_hash, sig, RSA_EXPONENT), "the signature does not verify");

    // The padded header: CRLF, the header, then whatever the storage holds.
    let mut h: [u8; HP] = [0; HP];
    h[0] = CR;
    h[1] = LF;
    for i in 0..MAX_HEADER {
        h[i + 2] = header.storage()[i];
    }
    let hend = hlen + 2;
    let (hp, hpw) = fingerprints(h);

    // 2. The DKIM-Signature field: the last field.
    let dkim_name: [u8; 17] = "\r\ndkim-signature:".as_bytes();
    let dw: [u8; 17] = window(h, dkim_start - 2);
    assert(dw == dkim_name, "no DKIM-Signature field at the hinted position");
    let region = dkim_start + 15;
    assert(region <= hend);
    // Tag starts: the first non-WSP byte after the field's colon or a `;`.
    let mut tag_start: [Field; HP] = [0; HP];
    // Flags as products (1 until i reaches the position, then 0). A running
    // sum would stay a symbolic linear expression and be multiplied out
    // term by term below: quadratic in the loop (measured: 914k gates for
    // one such flag over the header, 16k as a product).
    let mut before_dkim: Field = 1; // i < dkim_start
    let mut before_region: Field = 1; // i < region
    let mut before_end: Field = 1; // i < hend
    let mut after_semi: Field = 1;
    let mut n_d: Field = 0;
    let mut n_s: Field = 0;
    let mut n_t: Field = 0;
    let mut n_c: Field = 0;
    let mut n_bh: Field = 0;
    let mut n_l: Field = 0;
    for i in 0..HP - 2 {
        let c = h[i];
        before_dkim = before_dkim * (1 - eqf(i, dkim_start));
        before_region = before_region * (1 - eqf(i, region));
        before_end = before_end * (1 - eqf(i, hend));
        let past = 1 - before_dkim;
        // No CRLF from the DKIM-Signature field on (none after the header
        // either: it would only fail the proof).
        assert(past * cls(IS_CR, c) * cls(IS_LF, h[i + 1]) == 0, "the DKIM-Signature field is not the last");
        let inside = (1 - before_region) * before_end;
        let w = cls(WSP, c);
        let ts = inside * (1 - w) * after_semi;
        tag_start[i] = ts;
        after_semi = inside * (w * after_semi + (1 - w) * cls(IS_SEMI, c)) + (1 - inside);
        let eq1 = cls(IS_EQ, h[i + 1]);
        n_d += ts * cls(IS_D, c) * eq1;
        n_s += ts * cls(IS_S, c) * eq1;
        n_t += ts * cls(IS_T, c) * eq1;
        n_c += ts * cls(IS_C, c) * eq1;
        n_bh += ts * cls(IS_B, c) * cls(IS_H, h[i + 1]) * cls(IS_EQ, h[i + 2]);
        n_l += ts * cls(IS_L, c);
    }
    assert(n_l == 0, "the signature has an l= tag: it does not cover the whole body");
    assert(n_d == 1, "the signature needs exactly one d=");
    assert(n_s == 1, "the signature needs exactly one s=");
    assert(n_c == 1, "the signature must name its canonicalization (c=relaxed/...)");
    assert(n_bh == 1, "the signature needs exactly one bh=");

    // d= and s=: disclosed.
    assert((tag_start[d_tag] == 1) & (h[d_tag] == 100));
    let d_bytes: [u8; DOMAIN_BYTES] = assert_tag_value(h, d_tag + 1, d_value, domain_len, hend);
    pack_check(d_bytes, domain);
    assert((tag_start[s_tag] == 1) & (h[s_tag] == 115));
    let s_bytes: [u8; SELECTOR_BYTES] =
        assert_tag_value(h, s_tag + 1, s_value, selector_len, hend);
    pack_check(s_bytes, selector);

    // c=: header canonicalization relaxed.
    assert((tag_start[c_tag] == 1) & (h[c_tag] == 99) & (h[c_tag + 1] == EQ));
    let relaxed: [u8; 7] = "relaxed".as_bytes();
    let cv0 = if h[c_tag + 2] == SP { c_tag + 3 } else { c_tag + 2 };
    assert(cv0 + 7 <= hend);
    let cw: [u8; 8] = window(h, cv0);
    for j in 0..7 {
        assert(cls(LOWER, cw[j]) == relaxed[j] as Field, "the header canonicalization is not relaxed");
    }
    assert((cw[7] == 47) | (cw[7] == SEMI) | (cw[7] == SP) | (cv0 + 7 == hend));

    // t=: disclosed when present (0 when absent).
    if n_t == 0 {
        assert(sig_time == 0);
    } else {
        assert(n_t == 1, "the signature has several t= tags");
        assert((tag_start[t_tag] == 1) & (h[t_tag] == 116));
        let tv: [u8; 20] = assert_tag_value(h, t_tag + 1, t_tag + 2, t_len, hend);
        let mut t: Field = 0;
        let mut below: Field = 1;
        for j in 0..20 {
            below = below * (1 - eqf(j, t_len));
            let d = tv[j] as Field;
            assert(below * (1 - cls(DIGIT, tv[j])) == 0, "t= is not a number");
            t = pin(below * (t * 10 + d - 48) + (1 - below) * t);
        }
        assert(t == sig_time as Field);
    }

    // bh=: the body hash.
    assert((tag_start[bh_tag] == 1) & (h[bh_tag] == 98) & (h[bh_tag + 1] == 104));
    let bh_text: [u8; 44] = assert_tag_value(h, bh_tag + 2, bh_tag + 3, 44, hend);
    let body_hash: [u8; 32] = BASE64_DECODER::decode(bh_text);
    let computed: [u8; 32] = sha256_var(body.storage(), blen);
    assert(computed == body_hash, "the body does not match the signed body hash");

    // 3. Signed MIME fields and From. A field the counts find past the
    // header's end can't be the hinted one (each is bounded by `hend`).
    let ct_name: [u8; 15] = "\r\ncontent-type:".as_bytes();
    let cte_name: [u8; 28] = "\r\ncontent-transfer-encoding:".as_bytes();
    let from_name: [u8; 7] = "\r\nfrom:".as_bytes();
    let n_ct = count_const::<HP, 15, MAX_HEADER + 2>(hp, hpw, ct_name);
    let n_cte = count_const::<HP, 28, MAX_HEADER + 2>(hp, hpw, cte_name);
    let n_from = count_const::<HP, 7, MAX_HEADER + 2>(hp, hpw, from_name);
    assert(n_ct == 1, "the signed header needs exactly one Content-Type");
    assert(n_from == 1, "the signed header needs exactly one From");

    let ctw: [u8; 15] = window(h, ct_field - 2);
    assert(ctw == ct_name);
    let ct_bytes = unpack(content_type, content_type_len);
    assert_field_value(h, ct_field + 13, ct_bytes, content_type_len, hend);

    let cte_bytes = unpack(cte, cte_len);
    if n_cte == 0 {
        assert(cte_len == 0);
    } else {
        assert(n_cte == 1, "the signed header has several Content-Transfer-Encoding fields");
        assert(cte_len > 0);
        let ew: [u8; 28] = window(h, cte_field - 2);
        assert(ew == cte_name);
        assert_field_value(h, cte_field + 26, cte_bytes, cte_len, hend);
    }

    // From: is the address after its last `@` the signing domain or under it?
    let fw: [u8; 7] = window(h, from_field - 2);
    assert(fw == from_name);
    let fv: [u8; FROM_BYTES] = window(h, from_field + 5);
    let mut open: Field = 1; // before the field's CR
    let mut no_at: Field = 1;
    let mut last_at: Field = 0;
    let mut tail_end: Field = 0;
    for j in 0..FROM_BYTES {
        let c = fv[j];
        open = open * (1 - cls(IS_CR, c));
        let at = open * cls(IS_AT, c);
        no_at = no_at * (1 - at);
        last_at = pin(at * (j as Field) + (1 - at) * last_at);
        let keep = open * (1 - cls(FROM_STRIP, c));
        tail_end = pin(keep * ((j + 1) as Field) + (1 - keep) * tail_end);
    }
    let has_at = 1 - no_at;
    assert(open == 0, "the From field is longer than this circuit reads");
    let last_at = last_at as u32;
    let tail_end = tail_end as u32;
    // A `from:` field start inside the header is a real signed field
    // (relaxed canonicalization leaves CRLF only between fields), and it
    // ends before the DKIM-Signature field.
    assert(from_field + 5 < hend);
    let mut aligned = false;
    if (has_at == 1) & (tail_end > last_at + 1) {
        let dom_len = tail_end - last_at - 1;
        if dom_len >= domain_len {
            // `off` >= 1: the `@` is before it.
            let off = tail_end - domain_len;
            let mut fvp: [u8; FROM_BYTES + DOMAIN_BYTES] = [0; FROM_BYTES + DOMAIN_BYTES];
            for j in 0..FROM_BYTES {
                fvp[j] = fv[j];
            }
            let mut diff: Field = 0;
            let mut below: Field = 1;
            for j in 0..DOMAIN_BYTES {
                below = below * (1 - eqf(j, domain_len));
                let x = cls(LOWER, fvp[off + j]) - cls(LOWER, d_bytes[j]);
                // Any difference before the length makes `diff` non-zero:
                // squares cannot cancel, and 279 of them stay far below
                // the field's size.
                diff += below * x * x;
            }
            let boundary = (dom_len == domain_len) | (fvp[off - 1] == DOT);
            aligned = (diff == 0) & boundary;
        }
    }
    assert(aligned == from_aligned, "from_aligned is wrong");

    // 4. The body.
    let mut b: [u8; BP] = [0; BP];
    for i in 0..MAX_BODY {
        b[i] = body.storage()[i];
    }
    let (bp, bpw) = fingerprints(b);
    // No forwarded-message markers anywhere (quoted lines included).
    let markers = absent::<BP, 28, MAX_BODY>(bp, bpw, "---------- Forwarded message".as_bytes())
        * absent::<BP, 26, MAX_BODY>(bp, bpw, "-----Original Message-----".as_bytes())
        * absent::<BP, 23, MAX_BODY>(bp, bpw, "Begin forwarded message".as_bytes());
    assert(markers != 0, "a forwarded-message marker is in the body");

    // loose[i]: only loose bytes between the line start and i. And the
    // line feeds.
    let mut loose: [Field; MAX_BODY + 1] = [0; MAX_BODY + 1];
    let mut n_lf: Field = 0;
    let mut prev_open: Field = 1;
    let mut prev_lf: Field = 1;
    let mut prev_loose: Field = 0;
    for i in 0..MAX_BODY + 1 {
        let o = prev_lf + prev_open * prev_loose - prev_lf * prev_open * prev_loose;
        loose[i] = o;
        prev_open = o;
        prev_lf = cls(IS_LF, b[i]);
        prev_loose = cls(LOOSE, b[i]);
        if i < MAX_BODY {
            n_lf += prev_lf;
        }
    }

    // The lines: the prover lists where each starts and where its text
    // starts. Starts rise, each follows a line feed, and there is one per
    // line feed, so the list is every line (a line feed past the body's end
    // would need a start past it, which fails). Each text start is the one
    // position of its line past the loose bytes.
    assert(n_lines as Field == n_lf + 1, "the line list is not every line");
    assert(n_lines <= MAX_LINES, "the body has more lines than this circuit reads");
    let mut prev_start: u32 = 0;
    let mut listed: Field = 1; // j < n_lines
    for j in 0..MAX_LINES {
        listed = listed * (1 - eqf(j, n_lines));
        if bit(listed) {
            let ls = line_starts[j];
            if j == 0 {
                assert(ls == 0);
            } else {
                assert(ls > prev_start);
                assert(b[ls - 1] == LF);
            }
            let cs = content_starts[j];
            assert((cs >= ls) & (cs <= blen));
            // Inside its own line: before the next line's start.
            if j + 1 < MAX_LINES {
                if j + 1 != n_lines {
                    assert(cs < line_starts[j + 1]);
                }
            }
            assert(loose[cs] * (1 - cls(LOOSE, b[cs])) == 1, "a line's text start is wrong");
            prev_start = ls;
        }
    }

    let mut label_bytes: [[u8; LABEL_BYTES]; SLOTS] = [[0; LABEL_BYTES]; SLOTS];
    let mut label_fp: [Field; SLOTS] = [0; SLOTS];
    for k in 0..SLOTS {
        let lb = unpack(labels[k], label_lens[k]);
        label_bytes[k] = lb;
        // Bytes after the length are zero (`unpack`), so they add nothing.
        let mut acc: Field = 0;
        let mut r: Field = 1;
        for j in 0..LABEL_BYTES {
            acc += (lb[j] as Field) * r;
            r *= R;
        }
        label_fp[k] = pin(acc);
        if label_lens[k] > 0 {
            // An ASCII first byte, not a space and not `>`.
            assert(
                (lb[0] > 32) & (lb[0] < 127) & (lb[0] != GT),
                "a label must start with a visible ASCII character other than >",
            );
        }
    }

    // Each label, counted at the text start of every line that is not
    // quoted: every line `email_dkim` would read it from is counted.
    let mut counts: [Field; SLOTS] = [0; SLOTS];
    let mut listed: Field = 1;
    for j in 0..MAX_LINES {
        listed = listed * (1 - eqf(j, n_lines));
        let cs = content_starts[j];
        let open = listed * (1 - cls(IS_GT, b[cs]));
        let p0 = bp[cs];
        let w0 = bpw[cs];
        for k in 0..SLOTS {
            let z = bp[cs + label_lens[k]] - p0 - label_fp[k] * w0;
            counts[k] += open * ((z == 0) as Field);
        }
    }

    for k in 0..SLOTS {
        let len = label_lens[k];
        let vlen = value_lens[k];
        let vb = unpack(values[k], vlen);
        if len == 0 {
            assert(vlen == 0);
        } else {
            assert(counts[k] == 1, "a label starts more than one line outside quoted lines");
            // The label's line, read from its start: spaces or tabs, the
            // label at the line's text start (so counted above), spaces or
            // tabs, the value, spaces or tabs, CRLF, all inside the body.
            let li = label_line[k];
            assert(li < n_lines);
            let ls = line_starts[li];
            let at = content_starts[li];
            let line: [u8; LINE_BYTES] = window(b, ls);
            let ind = at - ls;
            let lend = ind + len;
            let v = value_at[k];
            assert(v >= at + len);
            let voff = v - ls;
            let vend = voff + vlen;
            assert(vlen > 0, "a label has no value");
            assert(vend + 2 <= LINE_BYTES, "a labelled line is longer than this circuit reads");
            let vw: [u8; VALUE_BYTES] = window(b, v);
            let mut below: Field = 1;
            for j in 0..VALUE_BYTES {
                below = below * (1 - eqf(j, vlen));
                assert(below * ((vw[j] as Field) - (vb[j] as Field)) == 0);
            }
            assert(cls(WSP, vb[0]) == 0);
            assert(cls(WSP, vb[vlen - 1]) == 0);
            let mut in_ind: Field = 1;
            let mut in_label: Field = 1;
            let mut in_gap: Field = 1;
            let mut in_value: Field = 1;
            let mut no_cr: Field = 1; // no CR after the value yet
            let mut not_done: Field = 1; // its LF not reached yet
            for j in 0..LINE_BYTES {
                let c = line[j];
                in_ind = in_ind * (1 - eqf(j, ind));
                in_label = in_label * (1 - eqf(j, lend));
                in_gap = in_gap * (1 - eqf(j, voff));
                in_value = in_value * (1 - eqf(j, vend));
                let wsp = cls(WSP, c);
                // Indentation: spaces or tabs.
                assert(in_ind * (1 - wsp) == 0);
                // The label.
                let lab = (1 - in_ind) * in_label;
                // j - ind inside the label, 0 elsewhere (no underflow).
                let idx = (lab * ((j as Field) - (ind as Field))) as u32;
                assert(lab * ((c as Field) - (label_bytes[k][idx] as Field)) == 0);
                // Between label and value: spaces or tabs.
                assert((1 - in_label) * in_gap * (1 - wsp) == 0);
                // The value: no CR or LF.
                assert((1 - in_gap) * in_value * cls(IS_CRLF_OR_ZERO, c) == 0);
                // After it: spaces or tabs, then CR, then LF.
                let tail = (1 - in_value) * not_done;
                let is_cr = cls(IS_CR, c);
                let is_lf = cls(IS_LF, c);
                let need_lf = 1 - no_cr;
                assert(tail * need_lf * (1 - is_lf) == 0, "a labelled line does not end with CRLF");
                assert(tail * no_cr * (1 - is_cr) * (1 - wsp) == 0, "text after a value on its line");
                no_cr = no_cr * (1 - tail * no_cr * is_cr);
                not_done = not_done * (1 - tail * need_lf);
            }
            assert(not_done == 0, "a labelled line does not end with CRLF");
            // That LF ends line `li` (no LF comes before it in the line), and
            // another line follows it: the whole line is inside the body.
            assert(li + 1 < n_lines, "a labelled line runs past the body");
        }
    }
}
