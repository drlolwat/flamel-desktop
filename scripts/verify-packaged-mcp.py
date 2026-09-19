#!/usr/bin/env python3
"""Smoke checks for an isolated packaged Desktop; never use a live user profile."""
import json, os, socket, sqlite3, struct, subprocess, tempfile, time
from pathlib import Path

import argparse, sys, threading, queue
parser = argparse.ArgumentParser(description="Exercise packaged Flamel MCPs using disposable files, SQLite, memory, Git, and a headless browser.")
parser.add_argument('--bin-dir', required=True, type=Path, help='Directory containing the six packaged MCP executables')
parser.add_argument('--runtime-file', type=Path, help='Optional cli-runtime.json from an isolated running Desktop profile; enables native callback and custom-settings checks')
args = parser.parse_args()
BINS = args.bin_dir.resolve()
ROOT = Path(tempfile.mkdtemp(prefix='flamel-packaged-mcp-'))
RUNTIME = json.loads(args.runtime_file.read_text()) if args.runtime_file else None
ENV = os.environ.copy()
for key in list(ENV):
    if key.startswith('FLAMEL_'): ENV.pop(key)
ENV.update(FLAMEL_USER_HOME=str(ROOT), FLAMEL_PROJECT_PATH=str(ROOT), FLAMEL_MEMORIES_DB=str(ROOT/'memories.db'), FLAMEL_BIN_DIR=str(BINS), GIT_WORK_DIR=str(ROOT))
if RUNTIME:
    ENV.update(FLAMEL_URL=RUNTIME['apiBase'], FLAMEL_TOKEN=RUNTIME['mcpToken'], FLAMEL_USER_ID=RUNTIME['userId'])
print('Disposable fixture directory:', ROOT, flush=True)

def api(method,path,body=None,expected_status=None):
    assert RUNTIME and RUNTIME['apiBase'].startswith('ipc:/'), 'Native settings check currently supports Unix socket profiles'
    s=socket.socket(socket.AF_UNIX);s.settimeout(30);s.connect(RUNTIME['apiBase'][4:])
    data=json.dumps(dict(version=1,method=method,path=path,headers={'x-flamel-token':RUNTIME['mcpToken']},body=body)).encode()
    s.sendall(struct.pack('!I',len(data))+data)
    def read(n):
        value=b''
        while len(value)<n:
            part=s.recv(n-len(value));assert part;value+=part
        return value
    head=json.loads(read(struct.unpack('!I',read(4))[0]));body=b''
    while True:
        size=struct.unpack('!I',read(4))[0]
        if not size:break
        body+=read(size)
    s.close();assert (head['status']==expected_status if expected_status is not None else head['status']<300),(path,head)
    return json.loads(body) if body else None

class MCP:
    def __init__(self,name,command=None):
        self.name=name;self.id=0
        self.log=(ROOT/(name+'.log')).open('w')
        self.p=subprocess.Popen(command or [str(BINS/(name + ('.exe' if os.name=='nt' else '')))],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=self.log,env=ENV,cwd=ROOT,bufsize=0)
        self.lines=queue.Queue()
        def read_lines():
            for line in self.p.stdout:self.lines.put(line)
            self.lines.put(b'')
        threading.Thread(target=read_lines,daemon=True).start()
        self.request('initialize',{'protocolVersion':'2024-11-05','capabilities':{},'clientInfo':{'name':'desktop-release-check','version':'1'}})
        self.p.stdin.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
        self.tools=self.request('tools/list',{})['tools']
    def request(self,method,params):
        self.id+=1;self.p.stdin.write((json.dumps({'jsonrpc':'2.0','id':self.id,'method':method,'params':params})+'\n').encode())
        deadline=time.monotonic()+120
        while time.monotonic()<deadline:
            try:line=self.lines.get(timeout=max(.1,deadline-time.monotonic()))
            except queue.Empty:raise AssertionError(self.name+' timeout')
            assert line,self.name+' exited'
            value=json.loads(line)
            if value.get('id')==self.id:
                assert 'error' not in value,(self.name,value)
                return value['result']
        raise AssertionError(self.name+' timeout')
    def call(self,name,args,contains=None):
        value=self.request('tools/call',{'name':name,'arguments':args})
        assert not value.get('isError'),(self.name,name,value)
        if contains:assert contains in json.dumps(value),(self.name,value)
        print(self.name+': '+name+' '+str(args.get('action',''))+' passed',flush=True)
        return value
    def close(self):
        self.p.stdin.close()
        try:self.p.wait(timeout=5)
        except subprocess.TimeoutExpired:self.p.terminate();self.p.wait(timeout=5)
        self.log.close()

def check_remote_integration():
    import secrets
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
    token=secrets.token_urlsafe(32);calls=[]
    class Handler(BaseHTTPRequestHandler):
     def log_message(self,*args):pass
     def do_GET(self):self.send_response(405);self.end_headers()
     def do_DELETE(self):self.send_response(204);self.end_headers()
     def do_POST(self):
      assert self.headers.get('Authorization')=='Bearer '+token
      r=json.loads(self.rfile.read(int(self.headers['Content-Length'])));method=r['method'];calls.append(method)
      if 'id' not in r:self.send_response(202);self.end_headers();return
      if method=='initialize':out={'protocolVersion':r['params']['protocolVersion'],'capabilities':{'tools':{}},'serverInfo':{'name':'remote-fixture','version':'1'}}
      elif method=='tools/list':out={'tools':[{'name':'echo','description':'Fixture','inputSchema':{'type':'object'}}]}
      elif method=='resources/list':out={'resources':[]}
      elif method=='prompts/list':out={'prompts':[]}
      elif method=='tools/call':out={'content':[{'type':'text','text':r['params']['arguments']['text']}]}
      else:out={}
      body=json.dumps({'jsonrpc':'2.0','id':r['id'],'result':out}).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
    s=ThreadingHTTPServer(('127.0.0.1',0),Handler);threading.Thread(target=s.serve_forever,daemon=True).start()
    entry=api('POST','/api/capabilities/catalog',{'slug':'remote-'+ROOT.name,'title':'Remote MCP fixture','transport':'streamable_http','remoteUrl':'http://127.0.0.1:'+str(s.server_port)+'/mcp','authKind':'token'})
    base='/api/capabilities/'+entry['id']
    try:
     api('POST',base+'/credential',{'value':token,'kind':'token'});api('POST',base+'/connect')
     probe=api('POST',base+'/test');assert probe['toolCount']==1
     result=api('POST',base+'/execute',{'toolName':'echo','arguments':{'text':'remote-mcp-authenticated-call-proof'}});assert 'remote-mcp-authenticated-call-proof' in json.dumps(result)
     assert token not in json.dumps(api('GET','/api/capabilities/catalog'))
     assert 'tools/call' in calls
     print('Packaged Desktop remote MCP: authenticated discovery and tool call passed; credential absent from catalog.')
    finally:
     try:
      api('DELETE',base+'/connect')
      api('DELETE',base+'/credential',expected_status=404)  # Disconnect already removed it.
     finally:s.shutdown();s.server_close()

if __name__=='__main__':
    subprocess.run(['git','init','-q',str(ROOT)],check=True)
    (ROOT/'fixture.txt').write_text('packaged-mcp-fixture\n')
    db=sqlite3.connect(ROOT/'fixture.db');db.execute('CREATE TABLE IF NOT EXISTS proof (value TEXT)');db.execute('DELETE FROM proof');db.execute('INSERT INTO proof VALUES (?)',('packaged-mcp-fixture',));db.commit();db.close()
    checks=[('db_mcp','db',{'action':'query','connection_string':str(ROOT/'fixture.db'),'query':'SELECT value FROM proof'},'packaged-mcp-fixture'),('memory_mcp','memory',{'action':'add','content':'packaged-mcp-fixture','project_scope':False},None),('git_mcp','git',{'action':'repo.status','path':str(ROOT)},'fixture.txt'),('docs_mcp','search_flamel_docs',{'query':'MCP','max_results':2},None)]
    for binary,tool,args,expected in checks:
        m=MCP(binary)
        try:
            m.call(tool,args,expected)
            if binary=='memory_mcp':m.call('memory',{'action':'search','query':'packaged-mcp-fixture'},'packaged-mcp-fixture')
        finally:m.close()
    m=MCP('flamel_mcp')
    try:
        m.call('file_read',{'file':'fixture.txt'},'packaged-mcp-fixture')
        m.call('file_edit',{'action':'create','relative_path':'gateway.txt','content':'gateway-mcp-proof'})
        m.call('file_read',{'file':'gateway.txt'},'gateway-mcp-proof')
        m.call('git',{'action':'repo.status','path':str(ROOT)},'fixture.txt')
        m.call('db',{'action':'query','connection_string':str(ROOT/'fixture.db'),'query':'SELECT value FROM proof'},'packaged-mcp-fixture')
        m.call('memory',{'action':'search','query':'packaged-mcp-fixture'},'packaged-mcp-fixture')
        m.call('search_flamel_docs',{'query':'MCP','max_results':1})
        m.call('browser_session',{'action':'open','url':'data:text/html,<h1>gateway-browser-proof</h1>'})
        m.call('browser_inspect',{'include':['snapshot']},'gateway-browser-proof')
        m.call('browser_session',{'action':'close'})
        if RUNTIME:
            assert api('GET','/api/me')['id']==RUNTIME['userId']
            m.call('plan',{'action':'list'})
    finally:m.close()
    m=MCP('browser_mcp')
    try:
        m.call('browser_session',{'action':'open','url':'data:text/html,<title>Packaged MCP proof</title><h1>packaged-mcp-fixture</h1>'})
        m.call('browser_inspect',{'include':['snapshot']},'packaged-mcp-fixture')
        m.call('browser_session',{'action':'close'})
    finally:m.close()

    if RUNTIME:
        fixture=ROOT/'custom-mcp.py'
        fixture.write_text("""import json, sys
assert sys.argv[1:]==['--proof','argument with spaces']
for line in sys.stdin:
    r=json.loads(line)
    if 'id' not in r:continue
    if r['method']=='initialize':out={'protocolVersion':'2024-11-05','capabilities':{'tools':{}},'serverInfo':{'name':'release-fixture','version':'1'}}
    elif r['method']=='tools/list':out={'tools':[{'name':'echo','description':'Release fixture','inputSchema':{'type':'object'}}]}
    else:out={'content':[{'type':'text','text':r['params']['arguments']['text']}]}
    print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':out}),flush=True)
""")
        saved=api('POST','/api/mcp',{'name':'release-fixture-'+ROOT.name,'command':sys.executable,'args':[str(fixture),'--proof','argument with spaces'],'projectPath':str(ROOT),'enabled':True})
        try:
            from urllib.parse import quote
            loaded=next(row for row in api('GET','/api/mcp?projectPath='+quote(str(ROOT),safe='')) if row['id']==saved['id'])
            assert loaded['args']==[str(fixture),'--proof','argument with spaces'] and loaded['enabled']
            m=MCP('custom-fixture',[loaded['command'],*loaded['args']])
            try:m.call('echo',{'text':'custom-config-proof'},'custom-config-proof')
            finally:m.close()
            api('PUT','/api/mcp/'+saved['id'],{**loaded,'enabled':False})
            assert not next(row for row in api('GET','/api/mcp') if row['id']==saved['id'])['enabled']
        finally:api('DELETE','/api/mcp/'+saved['id'])
        # Exercise Desktop's own MCP client, not only a subprocess launched by
        # this check. The catalog entry remains disconnected in the test profile.
        source=fixture.read_text().replace("    else:out={'content'", "    elif r['method']=='resources/list':out={'resources':[]}\n    elif r['method']=='prompts/list':out={'prompts':[]}\n    else:out={'content'")
        fixture.write_text(source)
        entry=api('POST','/api/capabilities/catalog',{'slug':'stdio-'+ROOT.name,'title':'Release MCP fixture','transport':'stdio','command':sys.executable,'args':[str(fixture),'--proof','argument with spaces'],'authKind':'none'})
        base='/api/capabilities/'+entry['id']
        try:
            api('POST',base+'/connect')
            assert api('POST',base+'/test')['toolCount']==1
            result=api('POST',base+'/execute',{'toolName':'echo','arguments':{'text':'desktop-spawned-mcp-proof'}})
            assert 'desktop-spawned-mcp-proof' in json.dumps(result)
            print('Desktop custom stdio: discovery and execution passed',flush=True)
        finally:api('DELETE',base+'/connect')
        check_remote_integration()
    print('Packaged MCP checks passed. No paid provider turn was invoked.',flush=True)
