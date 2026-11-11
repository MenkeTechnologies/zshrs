set -l x (echo a; echo b)
if set -q x[2]
    echo has-second
end
if not set -q x[3]
    echo no-third
end
