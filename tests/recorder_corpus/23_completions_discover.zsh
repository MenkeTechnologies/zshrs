# When fpath is set or appended, the recorder synthesises one
# `completion` event per `_*` file found in the new directory —
# matches what zinit-report's "Completions:" section surfaces per
# plugin. This corpus uses a fixture directory that ships in the
# corpus tree (tests/recorder_corpus/fake_fpath/) so the test is
# hermetic; adjust expected counts if files are added/removed there.
fpath+=(${0:h}/fake_fpath)
