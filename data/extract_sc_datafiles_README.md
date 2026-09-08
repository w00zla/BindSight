# Extracting Star Citizen config data

`extract_sc_datafiles.sh` pulls the generic SC config files BindSight needs out
of the game's `Data.p4k` archive:

- `defaultProfile.xml` — the action master list
- `keybinding_localization.xml` — input token labels
- `global.ini` — localized GUI strings

## Usage

```sh
./extract_sc_datafiles.sh <path/to/Data.p4k>
```

The files land in `extracted/`. 

Example p4k location: `…/StarCitizen/LIVE/Data.p4k`.

## StarBreaker

Extraction is done by **StarBreaker**, a third-party CLI that reads the `.p4k`
archive and converts CryXML to plain XML. The script expects the `starbreaker`
binary next to it.

- Source / releases: https://github.com/diogotr7/StarBreaker
