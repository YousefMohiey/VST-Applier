#!/bin/bash
# Upload the VST-Applier v1.4 build: gofile (full zip) + tmpfiles (split parts as backup)
cd /c/Users/Administrator/projects/vst-applier/publish || exit 1

echo "=== GOFILE FULL ZIP ==="
SERVER=$(curl -sS https://api.gofile.io/servers --max-time 30 | python -c "import sys,json; print(json.load(sys.stdin)['data']['servers'][0]['name'])")
echo "server: $SERVER"
curl -sS -F "file=@VstApplier_v1.4_win-x64.zip" "https://$SERVER.gofile.io/contents/uploadfile" --max-time 3000 -o gofile_v14.json -w "\nhttp=%{http_code} size=%{size_upload}\n"
python -c "import json; d=json.load(open('gofile_v14.json')); print('downloadPage:', d['data'].get('downloadPage')); print('md5:', d['data'].get('md5'))"

echo "=== TMPFILES PART 1 (app) ==="
curl -sS -F "file=@VstApplier_v1.4_app.zip" https://tmpfiles.org/api/v1/upload --max-time 1500 -w "\nhttp=%{http_code}\n"

echo "=== TMPFILES PART 2 (common) ==="
curl -sS -F "file=@VstApplier_v1.4_common.zip" https://tmpfiles.org/api/v1/upload --max-time 1500 -w "\nhttp=%{http_code}\n"

echo "UPLOADS_DONE"
