#!/bin/sh

cd /home/vachat-server/
export VACHAT_WWWROOT_DIR=/home/vachat-server/wwwroot
cmd="./vachat-server"
if test $# -gt 0; then
  case "$1" in
      *.toml)
        cmd="./vachat-server $@"
        break
        ;;
      --*)
        cmd="./vachat-server $@"
        break
        ;;
      *) cmd="$@";;
  esac
fi
echo $cmd
$cmd
# exec "$cmd"
