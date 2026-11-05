"""Opt-in synthetic local tool-surface check. No cloud model calls or real credentials."""
import json, os, pathlib, subprocess, tempfile, threading, http.server, queue, time, signal
assert subprocess.check_output(['claude','--version'],text=True).startswith('2.1.318 ')
captured=queue.Queue(); methods=[]
class Handler(http.server.BaseHTTPRequestHandler):
 def log_message(self,*a): pass
 def do_POST(self):
  body=json.loads(self.rfile.read(int(self.headers.get('Content-Length','0'))))
  if self.path=='/mcp':
   method=body.get('method');methods.append(method)
   if method and method.startswith('notifications/'):
    self.send_response(202);self.end_headers();return
   result={'protocolVersion':'2024-11-05','capabilities':{'tools':{}},'serverInfo':{'name':'synthetic-memory','version':'1'}} if method=='initialize' else {'tools':[{'name':n,'description':'Synthetic memory','inputSchema':{'type':'object','properties':{}}} for n in ['memory_search','memory_list','memory_read','memory_save','memory_edit','memory_forget']]} if method=='tools/list' else {}
   data=json.dumps({'jsonrpc':'2.0','id':body.get('id'),'result':result}).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(data);return
  if self.path.endswith('/count_tokens'):
   data=json.dumps({'input_tokens':1}).encode()
  else:
   names=[t.get('name',t.get('type')) for t in body.get('tools',[])];captured.put(names)
   data=json.dumps({'id':'synthetic','type':'message','role':'assistant','model':body.get('model'),'content':[{'type':'text','text':'Synthetic response.'}],'stop_reason':'end_turn','stop_sequence':None,'usage':{'input_tokens':1,'output_tokens':1}}).encode()
  self.send_response(200);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(data)
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler);threading.Thread(target=server.serve_forever,daemon=True).start()
with tempfile.TemporaryDirectory(prefix='discard-claude-smoke-') as root:
 root=pathlib.Path(root);config=root/'config';config.mkdir();cwd=root/'cwd';cwd.mkdir();mcp=root/'memory.json';mcp.write_text(json.dumps({'mcpServers':{'discard-memory':{'type':'http','url':f'http://127.0.0.1:{server.server_port}/mcp','headers':{'Authorization':'Bearer synthetic'}}}}))
 env=os.environ.copy();env.update(CLAUDE_CONFIG_DIR=str(config),ANTHROPIC_API_KEY='synthetic',ANTHROPIC_BASE_URL=f'http://127.0.0.1:{server.server_port}',CLAUDE_CODE_DISABLE_AUTO_MEMORY='1',CLAUDE_CODE_DISABLE_CLAUDE_MDS='1');env.pop('CLAUDE_CODE_PLUGIN_DIRS',None);env.pop('CLAUDECODE',None);env.pop('ANTHROPIC_AUTH_TOKEN',None);env.pop('CLAUDE_CODE_OAUTH_TOKEN',None)
 args=['claude','-p','--input-format','stream-json','--output-format','stream-json','--verbose','--permission-mode','manual','--permission-prompts','none','--tools','','--strict-mcp-config','--mcp-config',str(mcp),'--disable-slash-commands','--restricted','--system-prompt-snapshot','off','--model','claude-sonnet-4-6','--settings',json.dumps({'disableAllHooks':True,'autoMemoryEnabled':False,'env':{'CLAUDE_CODE_DISABLE_AUTO_MEMORY':'1','CLAUDE_CODE_DISABLE_CLAUDE_MDS':'1','CLAUDE_CODE_PLUGIN_DIRS':''},'pluginConfigs':{'cc-plugin-agents-md@builtin':{'options':{'instructionFiles':'managed-only'}}}})]
 process=subprocess.Popen(args,cwd=cwd,env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True,start_new_session=True)
 process.stdin.write(json.dumps({'type':'user','message':{'role':'user','content':'Synthetic test. Return a brief answer.'}})+'\n');process.stdin.flush()
 try:
  names=captured.get(timeout=30);print('CLAUDE_TOOL_SURFACE',json.dumps(names));print('MCP_METHODS',json.dumps(methods));allowed={'mcp__discard-memory__'+n for n in ['memory_search','memory_list','memory_read','memory_save','memory_edit','memory_forget']};assert set(names)<=allowed|{'EndConversation'},names;assert allowed<=set(names),names
 finally:os.killpg(process.pid,signal.SIGKILL);process.wait()
server.shutdown()
