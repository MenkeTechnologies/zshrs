# Nested subshells and pipelines
( ( ls | grep a ) | wc -l )
{ { echo hi | cat; } | wc -c; }
