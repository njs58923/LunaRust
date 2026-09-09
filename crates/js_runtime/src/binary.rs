use deno_core::op2;

#[op2]
#[buffer]
pub fn op_encode_utf8(#[string] value: String) -> Vec<u8> {
    value.into_bytes()
}

#[op2]
#[string]
pub fn op_decode_utf8(
    #[buffer] bytes: &[u8],
    fatal: bool,
    ignore_bom: bool,
) -> Result<String, anyhow::Error> {
    let bytes = if ignore_bom {
        bytes
    } else {
        bytes.strip_prefix(&[239, 187, 191]).unwrap_or(bytes)
    };
    if fatal {
        Ok(std::str::from_utf8(bytes)?.to_owned())
    } else {
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}
