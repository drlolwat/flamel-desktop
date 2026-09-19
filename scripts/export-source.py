#!/usr/bin/env python3
"""Export only audited public source. Never copy Git history or ignored files."""
from pathlib import Path
import argparse,gzip,hashlib,json,subprocess,tarfile
root=Path(__file__).resolve().parent.parent
parser=argparse.ArgumentParser();parser.add_argument('output',type=Path);args=parser.parse_args()
subprocess.run(['python3',str(root/'scripts/audit-source.py')],cwd=root,check=True)
files=json.loads((root/'scripts/source-allowlist.json').read_text())['files']
args.output.parent.mkdir(parents=True,exist_ok=True)
with args.output.open('wb') as raw, gzip.GzipFile(filename='',mode='wb',fileobj=raw,mtime=0) as compressed, tarfile.open(fileobj=compressed,mode='w') as archive:
 for name in sorted(files):
  p=root/name
  if p.is_symlink():raise SystemExit('Refusing source symlink: '+name)
  info=archive.gettarinfo(str(p),arcname='flamel-runtime/'+name)
  info.uid=info.gid=0;info.uname=info.gname='';info.mtime=0
  with p.open('rb') as source:archive.addfile(info,source)
checksum=hashlib.sha256(args.output.read_bytes()).hexdigest()
args.output.with_suffix(args.output.suffix+'.sha256').write_text(checksum+'  '+args.output.name+'\n')
print('Exported',len(files),'public source files; SHA-256',checksum)
