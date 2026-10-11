# Third-party notices

zshrs is released under the MIT license ([LICENSE](LICENSE)). Parts of it are
derived from, or distribute files taken from, other projects. Those parts
remain under their original terms, reproduced below. This file must ship with
every copy of the source and with every binary distribution.

## The Z Shell (zsh)

**Derived material.** `src/ported/` is a function-by-function port of the zsh C
source (`Src/*.c`, `Src/Zle/*.c`, `Src/Modules/*.c`, `Src/*.h`). Each ported
item cites the C file and line it follows (`c:NNN`). The port keeps zsh's
function names, structures, constants, control flow and many of its comments,
so it is a derivative work of zsh and is distributed under zsh's licence in
addition to the MIT grant on zshrs's own code.

**Bundled material.** `vendor/zsh/` (shell functions, completion functions,
help files, man pages, info pages) is zsh's own and is packed into the zshrs
binary at build time. Each file stays under the terms of its own header where
present and otherwise under the zsh licence below.

Licence text, copied verbatim from zsh's `LICENCE`:

```
The Z Shell is copyright (c) 1992-2017 Paul Falstad, Richard Coleman,
Zoltán Hidvégi, Andrew Main, Peter Stephenson, Sven Wischnowsky, and
others.  All rights reserved.  Individual authors, whether or not
specifically named, retain copyright in all changes; in what follows, they
are referred to as `the Zsh Development Group'.  This is for convenience
only and this body has no legal status.  The Z shell is distributed under
the following licence; any provisions made in individual files take
precedence.

Permission is hereby granted, without written agreement and without
licence or royalty fees, to use, copy, modify, and distribute this
software and to distribute modified versions of this software for any
purpose, provided that the above copyright notice and the following
two paragraphs appear in all copies of this software.

In no event shall the Zsh Development Group be liable to any party for
direct, indirect, special, incidental, or consequential damages arising out
of the use of this software and its documentation, even if the Zsh
Development Group have been advised of the possibility of such damage.

The Zsh Development Group specifically disclaim any warranties, including,
but not limited to, the implied warranties of merchantability and fitness
for a particular purpose.  The software provided hereunder is on an "as is"
basis, and the Zsh Development Group have no obligation to provide
maintenance, support, updates, enhancements, or modifications.
```

### GPL-licensed files bundled from zsh

zsh's own `LICENCE` states that certain shell functions are licensed under the
GNU General Public Licence and that anyone distributing a binary that includes
them must take that into account. The files under `vendor/zsh/functions/` that
carry such a header are:

| File | Terms stated in the file |
| --- | --- |
| `_darcs` | GNU GPL, version 2 or (at your option) any later version |
| `_zypper` | GPLv2 |
| `_qdbus` | GPLv2 |
| `_osc` | GPLv2 |

These files are separate, independently licensed works that zshrs packs as
data and does not link against. Their copyright holders are named in each
file's header. A distributor who omits them loses only those four completions;
none of the core functions are affected.

## fish-shell

`vendor/fish/` is a reference copy of fish-shell source (GPL version 2, see the
fish `COPYING`). It is a porting reference only: it is excluded from the
published crate (`Cargo.toml` `exclude`) and no build step reads it.

## Dependencies

Rust crate dependencies are used under their own licences, listed in
`Cargo.lock` and in each crate's published metadata.
