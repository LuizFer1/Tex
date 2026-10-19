ln -sf ../lib/tex/tex /usr/bin/tex
if [ -d /run/systemd/system ]; then
    systemctl daemon-reload || :
    systemctl enable tex-indexer.service || :
    systemctl restart tex-indexer.service || :
fi
