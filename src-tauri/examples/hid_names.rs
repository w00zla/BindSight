//! Verification spike: dump the HID `product_string` (iProduct descriptor) for
//! every connected HID device. Run with:
//!
//!     cargo run --example hid_names
//!
//! The hypothesis: `product_string` is exactly the name Star Citizen /
//! DirectInput shows (both Windows and Wine populate tszProductName from the
//! HID product string), unlike SDL which shows the evdev name. Compare the
//! output below against what SC displays for the same device.

fn main() -> Result<(), String> {
    let api = hidapi::HidApi::new().map_err(|e| e.to_string())?;

    for dev in api.device_list() {
        let vid = dev.vendor_id();
        let pid = dev.product_id();
        let product = dev.product_string().unwrap_or("<none>");
        let manufacturer = dev.manufacturer_string().unwrap_or("<none>");
        let serial = dev.serial_number().unwrap_or("<none>");

        println!("{vid:04x}:{pid:04x}  product={product:?}");
        println!("            manufacturer={manufacturer:?}  serial={serial:?}");
        println!("            usage_page={:#06x} usage={:#06x}", dev.usage_page(), dev.usage());
    }

    Ok(())
}
