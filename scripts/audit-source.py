#!/usr/bin/env python3
"""Check the reviewed source allowlist, classification, and public dependency boundary."""
from pathlib import Path
import hashlib,ipaddress,json,re,subprocess,sys,tomllib
root=Path(__file__).resolve().parent.parent
manifest=json.loads((root/'scripts/source-allowlist.json').read_text())
allowed=set(manifest['files'])
git_root=subprocess.run(['git','rev-parse','--show-toplevel'],cwd=root,capture_output=True,text=True)
has_git=git_root.returncode==0 and Path(git_root.stdout.strip()).resolve()==root
if has_git:
 files=set(subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard','-z'],cwd=root).decode().split('\0'))-{''}
else:
 # Source archives deliberately have no Git history. Do not accidentally use
 # a parent checkout's index when verifying an unpacked source distribution.
 import os
 files=set()
 for directory,dirs,names in os.walk(root):
  relative=Path(directory).relative_to(root)
  dirs[:]=[d for d in dirs if d not in {'.git','target','node_modules','dist','artifacts','__pycache__'} and (relative/d).as_posix() not in {'desktop/resources/bin','desktop/resources/notices','desktop/gen'}]
  for name in names:
   if not name.endswith('.tsbuildinfo'):
    files.add((relative/name).as_posix())
issues=[]
def report(name,reason):issues.append({'file':name,'reason':reason})
for name in sorted(files-allowed):report(name,'not in reviewed source allowlist')
for name in sorted(allowed-files):report(name,'allowlisted source missing')
private_parts={'cloud','forge','deploy','.env','node_modules','server','actions-compat','accounts','goal_worker','runner','personal_assistant','personal_assistant_chattle','personal_assistant_channels','personal_assistant_control','personal_assistant_delegation','middleware'}
private_files={'local_auth.rs','session_cache.rs','flamel_registration.rs','flamel_sso.rs','flamel_proxy.rs','flamel_jobs.rs','flamel_assistant.rs','flamel_assistant_runtime.rs','runners.rs','mariadb.rs','sandbox.rs','flamel_runner.rs','flamel_actions_runner.rs','job_capability_mcp.rs'}
for name in sorted(files):
 p=root/name
 if p.is_symlink():report(name,'source symlinks are forbidden');continue
 if name.startswith('.flamel/') and not (name.startswith('.flamel/workflows/') and name in allowed):report(name,'private Flamel metadata')
 if set(p.relative_to(root).parts)&private_parts or p.name in private_files:report(name,'private or operational source path')
 if name not in manifest.get('classification',{}):report(name,'missing source classification')
 try:text=p.read_text()
 except UnicodeError:continue
 if re.search(r'(?:from\s+[\'"]|path\s*=\s*"|include_(?:str|bytes)!\(\s*")[^\n]*(?:\.\./)+(?:cloud|forge|deploy|flamel-extraction-integration)',text):report(name,'public source imports private source')
 if re.search(r'-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----',text) and name not in {'fixtures/runtime-catalog/fixture-private-key.pem','engine/src/compaction.rs'}:report(name,'unexpected embedded private key')
 if re.search(r'(?:ghp_|github_pat_)[A-Za-z0-9_]{30,}|AKIA[0-9A-Z]{16}',text):report(name,'possible credential requires review')
 if str(Path.home())+'/' in text:report(name,'personal filesystem path')
 for address in re.findall(r'(?<![\d.])(?:\d{1,3}\.){3}\d{1,3}(?![\d.])',text):
  try:ip=ipaddress.IPv4Address(address)
  except ipaddress.AddressValueError:continue
  octets=ip.packed
  if octets[0]==10 or (octets[0]==172 and 16<=octets[1]<=31) or octets[:2]==bytes((192,168)):
   report(name,'private network address; use a documentation address in examples and tests')
 if p.name=='Cargo.toml':
  def walk(value):
   if isinstance(value,dict):
    for key,entry in value.items():
     if key=='path' and isinstance(entry,str) and not (p.parent/entry).resolve().is_relative_to(root):report(name,'Cargo path escapes public source')
     walk(entry)
   elif isinstance(value,list):
    for entry in value:walk(entry)
  walk(tomllib.loads(text))
 if name.startswith('engine/src/') and re.search(r'pub (?:mod (?:accounts|runner|local_auth|middleware)|struct (?:FlamelClaims|HostedExecution))\b',text):report(name,'private implementation symbol in public engine')
license_hash=hashlib.sha256((root/'LICENSE.md').read_bytes()).hexdigest()
if license_hash!=manifest['licenseSha256']:report('LICENSE.md','license differs from reviewed candidate policy')
for package in ['frontend','shared']:
 if (root/package/'LICENSE.md').read_bytes()!=(root/'LICENSE.md').read_bytes():report(package+'/LICENSE.md','package license differs')
remotes=subprocess.check_output(['git','remote'],cwd=root).decode().splitlines() if has_git else []
print(json.dumps({'sourceFiles':len(files),'licenseSha256':license_hash,'gitRemotes':remotes,'issues':issues,'result':'pass' if not issues else 'fail'},indent=2))
sys.exit(bool(issues))
