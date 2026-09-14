fn main() {
    // The bundled image-maps are compiled in (`imagemap::BUNDLED`); without
    // this, an edited map would not trigger a rebuild.
    println!("cargo:rerun-if-changed=resources/imagemaps");
    tauri_build::build()
}
