#!/usr/bin/env python3
"""Socket bridge for ghit drive-mode e2e.

Holds the one Unix-socket session the app serves, forwards command
lines read from a fifo to the server, and materializes the latest
frame rows to a pane file that the bash assertions grep like they
used to grep `tmux capture-pane -p`.

Usage:
    drive.py --sock PATH --cmd FIFOPATH --pane PANEFIL [--log LOGFILE]

Commands on the fifo are JSON lines of the drive protocol, e.g.
    {"type": "key", "key": "j"}
    {"type": "mouse", "kind": "down-left", "column": 4, "row": 25}
    {"type": "quit"}
Frames on the socket are JSON lines; the pane file holds the latest
frame's rows joined by newlines.
"""

import argparse
import json
import os
import socket
import sys
import threading
import time


def parse_args():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sock", required=True)
    ap.add_argument("--cmd", required=True)
    ap.add_argument("--pane", required=True)
    ap.add_argument("--log", default=None)
    return ap.parse_args()


def main():
    args = parse_args()
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    for _ in range(200):
        try:
            sock.connect(args.sock)
            break
        except OSError:
            time.sleep(0.05)
    else:
        print("drive.py: cannot connect to %s" % args.sock, file=sys.stderr)
        return 1
    reader = sock.makefile("r", encoding="utf-8")
    hello = reader.readline()
    try:
        msg = json.loads(hello)
        assert msg.get("type") == "hello", hello
    except Exception as exc:
        print("drive.py: bad hello %r (%s)" % (hello, exc), file=sys.stderr)
        return 1

    stop = threading.Event()

    def pump():
        # The fifo stays open for the session; bash holds the write end.
        with open(args.cmd, "r", encoding="utf-8") as fifo:
            for line in fifo:
                if stop.is_set():
                    return
                if not line.strip():
                    continue
                try:
                    sock.sendall((line if line.endswith("\n") else line + "\n").encode())
                except OSError:
                    return

    thread = threading.Thread(target=pump, daemon=True)
    thread.start()

    log = open(args.log, "a", encoding="utf-8") if args.log else None
    try:
        for line in reader:
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            if not isinstance(msg, dict) or msg.get("type") != "frame":
                continue
            if log is not None:
                log.write(line)
                log.flush()
            text = "\n".join(msg.get("rows", [])) + "\n"
            tmp = args.pane + ".tmp"
            with open(tmp, "w", encoding="utf-8") as fh:
                fh.write(text)
            os.replace(tmp, args.pane)
            if msg.get("quit"):
                break
    finally:
        stop.set()
        if log is not None:
            log.close()
        try:
            sock.close()
        except OSError:
            pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
