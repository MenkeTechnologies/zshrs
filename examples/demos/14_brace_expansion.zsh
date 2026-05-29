#!/usr/bin/env zshrs
# Brace expansion — comma-list, numeric range, letter range, stepped, zero-pad.

echo "── comma list ──"
for x in {red,green,blue}; do echo $x; done

echo "── numeric range ──"
echo {1..5}

echo "── numeric range descending ──"
echo {5..1}

echo "── stepped range ──"
echo {0..20..5}

echo "── zero-padded ──"
echo {01..10}

echo "── letter range ──"
echo {a..f}

echo "── cross product ──"
echo {a,b}{1,2,3}

echo "── nested ──"
echo {{a,b},{c,d}}

echo "── prefix/suffix ──"
echo file_{01..05}.txt
