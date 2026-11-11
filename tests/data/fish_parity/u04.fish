path mtime /etc/hosts | string match -qr "^\\d+\$"; echo $status
path mtime -R /etc/hosts | string match -qr "^\\d+\$"; echo $status
path mtime /nonexistent_zz; echo $status
