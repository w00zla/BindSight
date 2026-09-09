# Bundled hardware profiles

One folder per profile, named by the profile id (a UUID): `<id>/profile.json`
plus the image files it references by bare file name (png/jpg/jpeg/webp).
`profile.json` is format 1 — see `src/hwprofile.rs` for the schema. A user
profile with the same id (in the app data dir) shadows the bundled one.
