# Bundled palettes

Fastsonic puts these eight palettes in the themes folder the first time it
starts, as ordinary files to use, change or delete (see
[Custom themes](../../docs/_reference/settings-and-files.md#custom-themes)).
Nothing rewrites them afterwards: `.installed-palettes` in the folder records
which have been put there, so a deleted one stays deleted.

They are copied unchanged from `crates/fastframe-theme/themes/` in
[fastframe](https://github.com/crmne/fastframe) 0.1.5 (commit `027ce53`), by
Carmine Paolino, the author of Fastpotify, under the MIT License. The notice is
in [`THIRD-PARTY.md`](../../THIRD-PARTY.md).

Each sets the fourteen colours a palette usually changes and inherits
`overlay` and `shadow` from its `base`, in the format described in the guide.
