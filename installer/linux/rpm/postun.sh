if [ "$1" -eq 0 ]; then
    rm -f /usr/bin/tex
fi
if [ -d /run/systemd/system ]; then
    systemctl daemon-reload || :
fi
