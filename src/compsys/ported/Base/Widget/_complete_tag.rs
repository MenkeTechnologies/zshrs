//! Port of `_complete_tag` from `Completion/Base/Widget/_complete_tag`.
//!
//! Full upstream body (62 lines verbatim):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xt
//! sh: 2
//! sh: 3  # Complete tags using either TAGS or tags.  Looks up your directory
//! sh: 4  # hierarchy to find one.  If both exist, uses TAGS.
//! sh: 5  #
//! sh: 6  # You can override the choice of tags file with $TAGSFILE (for TAGS)
//! sh: 7  # or $tagsfile (for tags).
//! sh: 8  #
//! sh: 9  # Could be rewritten by some sed expert to use sed instead of perl.
//! sh:10
//! sh:11  emulate -L zsh
//! sh:12
//! sh:13  # Tags file to look for
//! sh:14  local c_Tagsfile=${TAGSFILE:-TAGS} c_tagsfile=${tagsfile:-tags} expl
//! sh:15  # Max no. of directories to scan up through
//! sh:16  integer c_maxdir=10
//! sh:17  # Context.
//! sh:18  local curcontext="$curcontext"
//! sh:19  local -a c_tags_array
//! sh:20
//! sh:21  if [[ -z "$curcontext" ]]; then
//! sh:22    curcontext="complete-tag:::"
//! sh:23  else
//! sh:24    curcontext="complete-tag:${curcontext#*:}"
//! sh:25  fi
//! sh:26
//! sh:27  local c_path=
//! sh:28  integer c_idir
//! sh:29  while [[ ! -f $c_path$c_Tagsfile &&
//! sh:30           ! -f $c_path$c_tagsfile && $c_idir -lt $c_maxdir ]]; do
//! sh:31    (( c_idir++ ))
//! sh:32    c_path=../$c_path
//! sh:33  done
//! sh:34
//! sh:35  if [[ -f $c_path$c_Tagsfile && $c_path$c_Tagsfile -ef $c_path$c_tagsfile &&
//! sh:36        "$(head -1 $c_path$c_tagsfile)" == '!_TAG_'* ]]; then
//! sh:37          c_Tagsfile=
//! sh:38  fi
//! sh:39
//! sh:40  if [[ -f $c_path$c_Tagsfile ]]; then
//! sh:41    # prefer the more comprehensive TAGS, which unfortunately is a
//! sh:42    # little harder to parse.
//! sh:43    # could do this with sed, just can't be bothered to work out how,
//! sh:44    # after quarter of an hour of trying, except for
//! sh:45    #  rm -f =sed; ln -s /usr/local/bin/perl /usr/bin/sed
//! sh:46    # but that's widely regarded as cheating.
//! sh:47    c_tags_array=($(sed -n \
//! sh:48          -e 's/^\(.*[a-zA-Z_0-9]\)[[ '$'\t'':;,()]*'$'\177''.*$/\1/' \
//! sh:49          -e 's/^.*[^a-zA-Z_0-9]//' \
//! sh:50          -e '/^[a-zA-Z_].*/p' $c_path$c_Tagsfile))
//! sh:51  #  c_tags_array=($(perl -ne '/([a-zA-Z_0-9]+)[ \t:;,\(]*\x7f/ &&
//! sh:52  #                  print "$1\n"' $c_path$c_Tagsfile))
//! sh:53    _main_complete - '' _wanted etags expl 'emacs tag' \
//! sh:54        compadd -a c_tags_array
//! sh:55  elif [[ -f $c_path$c_tagsfile ]]; then
//! sh:56    # tags doesn't have as much in, but the tag is easy to find.
//! sh:57    # we can use awk here.
//! sh:58    c_tags_array=($(awk '{ print $1 }' $c_path$c_tagsfile))
//! sh:59    _main_complete - '' _wanted vtags expl 'vi tag' compadd -a c_tags_array
//! sh:60  else
//! sh:61    return 1
//! sh:62  fi
//! ```

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
