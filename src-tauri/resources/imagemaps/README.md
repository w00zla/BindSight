# Bundled image-maps

One folder per image-map, named by its id (a UUID): `<id>/imagemap.json`
plus the one image file it references by bare file name (png/jpg/jpeg/webp).
`imagemap.json` is format 3 — see `src/imagemap.rs` for the schema. Bundled
image-maps are read-only in the app; users clone them.
