#!/bin/bash
# usage: dl.sh i start end
i=$1; s=$2; e=$3; f=parts/p$i
URL=https://download.dameng.com/eco/dm8/dm8_20241022_x86_rh6_64_single.tar
want=$((e-s+1))
while true; do
  have=$(stat -c %s $f 2>/dev/null || echo 0)
  [ $have -ge $want ] && exit 0
  curl -sS -r $((s+have))-$e $URL >> $f
  sleep 1
done
