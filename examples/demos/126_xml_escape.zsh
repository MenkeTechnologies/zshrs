#!/usr/bin/env zshrs
# XML/HTML entity escape + unescape.

xml_escape() {
    local s=$1
    s=${s//&/\&amp;}
    s=${s//</\&lt;}
    s=${s//>/\&gt;}
    s=${s//\"/\&quot;}
    s=${s//\'/\&apos;}
    echo "$s"
}

xml_unescape() {
    local s=$1
    s=${s//&amp;/&}
    s=${s//&lt;/<}
    s=${s//&gt;/>}
    s=${s//&quot;/\"}
    s=${s//&apos;/\'}
    echo "$s"
}

echo "── escape ──"
xml_escape "<div class=\"main\">Hello & World</div>"
xml_escape "5 < 10 && 10 > 5"
xml_escape "It's 'quoted' & has \"both\""

echo "── round-trip ──"
for s in "<a href='foo'>X</a>" "if (x < y && y > z)" "100% & 50%"; do
    enc=$(xml_escape "$s")
    dec=$(xml_unescape "$enc")
    if [[ "$s" == "$dec" ]]; then
        echo "OK: '$s'"
    else
        echo "FAIL: orig='$s' enc='$enc' dec='$dec'"
    fi
done

echo "── build a simple XML doc ──"
build_xml() {
    local tag=$1
    shift
    local content="$*"
    local escaped=$(xml_escape "$content")
    echo "<${tag}>${escaped}</${tag}>"
}

build_xml message "Hello & welcome!"
build_xml note "Pay <100% > 50% off"
build_xml code "if (a < b) return c;"
