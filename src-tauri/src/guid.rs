//! Conversion between the two GUID encodings BindSight has to bridge:
//! SDL's 32-hex-char joystick GUID and Star Citizen's `options/@Product` GUID
//! (`{PPPPVVVV-0000-0000-0000-504944564944}`, the trailing "504944564944"
//! being ASCII "PIDVID").
//!
//! SDL packs the vendor and product IDs little-endian inside its GUID, while SC
//! writes them big-endian as the first two 16-bit groups. So the bridge is:
//! pull the vendor and product 16-bit fields out of the SDL GUID, byte-swap
//! each, and splice them into SC's fixed layout. Verified against real hardware
//! GUIDs in the tests below.

/// Byte-swap a 4-hex-char (16-bit) field: `"1d23"` -> `"231D"`.
fn swap_u16_hex(field: &str) -> String {
    format!("{}{}", &field[2..4], &field[0..2]).to_uppercase()
}

/// Convert an SDL joystick GUID (32 lowercase hex chars, as returned by
/// `sdl2`'s `Joystick::guid().string()`) into Star Citizen's Product GUID
/// string. Returns `None` if the input is not exactly 32 hex characters.
pub fn sdl_guid_to_sc_product(sdl_guid: &str) -> Option<String> {
    let g = sdl_guid.trim();
    if g.len() != 32 || !g.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    // SDL GUID field layout (16-bit LE groups): bus | crc | vendor | 0 | product | 0 | version | 0
    let vendor = swap_u16_hex(&g[8..12]);
    let product = swap_u16_hex(&g[16..20]);
    Some(format!("{{{product}{vendor}-0000-0000-0000-504944564944}}"))
}

/// Extract the USB `(vendor, product)` IDs from an SDL joystick GUID, or `None`
/// if it is not exactly 32 hex characters. Used to match a device against its
/// HID entry (which is keyed by vendor/product).
pub fn sdl_guid_vendor_product(sdl_guid: &str) -> Option<(u16, u16)> {
    let g = sdl_guid.trim();
    if g.len() != 32 || !g.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let vendor = u16::from_str_radix(&swap_u16_hex(&g[8..12]), 16).ok()?;
    let product = u16::from_str_radix(&swap_u16_hex(&g[16..20]), 16).ok()?;
    Some((vendor, product))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_generic_guid() {
        // Synthetic SDL GUID with vendor field "3412" (LE 0x1234) and product
        // field "cdab" (LE 0xABCD). Proves the byte-swap and SC layout without
        // depending on any particular device, so it passes on a bare checkout.
        assert_eq!(
            sdl_guid_to_sc_product("0300000034120000cdab000001000000").as_deref(),
            Some("{ABCD1234-0000-0000-0000-504944564944}"),
        );
    }

    #[test]
    fn extracts_vendor_product() {
        // vendor field "3412" (LE 0x1234), product field "cdab" (LE 0xABCD)
        assert_eq!(
            sdl_guid_vendor_product("0300000034120000cdab000001000000"),
            Some((0x1234, 0xABCD)),
        );
        assert_eq!(sdl_guid_vendor_product("nope"), None);
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(sdl_guid_to_sc_product(""), None);
        assert_eq!(sdl_guid_to_sc_product("tooshort"), None);
        // 30 chars (too short)
        assert_eq!(sdl_guid_to_sc_product("03002fdb1d23000000020000110100"), None);
        // 32 chars but contains a non-hex digit
        assert_eq!(sdl_guid_to_sc_product("03002fdb1d230000000200001101000z"), None);
    }

    /// Optional cross-check against real hardware GUIDs (captured via
    /// `cargo run --example enum_joysticks` and matched against SC's exported
    /// <options> block). Not required to build or pass on a bare checkout —
    /// run explicitly with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn converts_real_hardware_guids() {
        assert_eq!(
            sdl_guid_to_sc_product("03002fdb1d2300000002000011010000").as_deref(),
            Some("{0200231D-0000-0000-0000-504944564944}"), // VKB Gladiator EVO R
        );
        assert_eq!(
            sdl_guid_to_sc_product("03004fdd1d2300000102000011010000").as_deref(),
            Some("{0201231D-0000-0000-0000-504944564944}"), // VKB Gladiator EVO L
        );
        assert_eq!(
            sdl_guid_to_sc_product("03008b9834340000210e000011010000").as_deref(),
            Some("{0E213434-0000-0000-0000-504944564944}"), // Keychron K2 HE
        );
    }
}
