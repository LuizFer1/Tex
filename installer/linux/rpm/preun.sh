if [ "$1" -eq 0 ] && [ -d /run/systemd/system ]; then
    systemctl disable --now tex-indexer.service || :
fi
