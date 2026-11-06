"""Opt-in Codex tool-surface check. Uses local endpoints and no cloud model calls."""
import argparse, json, os, pathlib, subprocess, tempfile, threading, http.server, queue, time, re


def require_controls(actual, expected):
    """Fail before thread creation if any required effective control differs."""
    mismatches = []

    def compare(found, required, path):
        if isinstance(required, dict):
            found = found if isinstance(found, dict) else {}
            for key, value in required.items():
                compare(found.get(key), value, path + [key])
        elif found != required:
            mismatches.append(".".join(path))

    compare(actual, expected, [])
    if mismatches:
        raise AssertionError("Required controls differ: " + ", ".join(mismatches))


def main():
    parser = argparse.ArgumentParser(description="Check Codex conversation tools with local synthetic endpoints.")
    parser.add_argument("--codex", default=os.environ.get("DISCARD_CODEX_BIN", "codex"), help="Codex CLI path (default: PATH)")
    parser.add_argument("--codex-home", type=pathlib.Path, default=pathlib.Path(os.environ.get("CODEX_HOME", str(pathlib.Path.home()/".codex"))), help="Source home for model metadata only; authentication is not copied")
    options = parser.parse_args()
    assert subprocess.check_output([options.codex,'--version'],text=True,timeout=10).strip() == 'codex-cli 0.166.0'
    repo = pathlib.Path(__file__).resolve().parents[4]
    source=(repo/'crates/discard-providers/src/conversation.rs').read_text()
    features=re.search(r'for name in \[(.*?)\] \{',source,re.S)[1]
    controls={'web_search':'disabled','agents':{'enabled':False},'memories':{'use_memories':False,'dedicated_tools':False,'generate_memories':False},'cloud':{'skills':{'enabled':False}},'skills':{'include_instructions':False,'bundled':{'enabled':False}},'project_doc_max_bytes':0,'tools':{'experimental_request_user_input':{'enabled':False},'update_plan':{'enabled':False}},'features':{n:False for n in re.findall(r'"([^"]+)"',features)}}
    controls['features']['skip_host_skill_discovery']=True
    captured=queue.Queue(); unexpected_mcp=[]
    class Handler(http.server.BaseHTTPRequestHandler):
     def log_message(self,*a): pass
     def do_POST(self):
      body=json.loads(self.rfile.read(int(self.headers.get('Content-Length','0'))))
      if self.path=='/forbidden':
       unexpected_mcp.append(self.path);self.send_response(500);self.end_headers();return
      if self.path=='/mcp':
       method=body.get('method')
       if method and method.startswith('notifications/'):
        self.send_response(202);self.end_headers();return
       result={'protocolVersion':'2024-11-05','capabilities':{'tools':{}},'serverInfo':{'name':'synthetic-memory','version':'1'}} if method=='initialize' else {'tools':[{'name':n,'description':'Synthetic memory','inputSchema':{'type':'object','properties':{}}} for n in ['memory_search','memory_list','memory_read','memory_save','memory_edit','memory_forget']]} if method=='tools/list' else {}
       data=json.dumps({'jsonrpc':'2.0','id':body.get('id'),'result':result}).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(data);return
      captured.put({'tools':body.get('tools') or [t for item in body.get('input',[]) for t in item.get('tools',[])], 'metadata':body.get('client_metadata')})
      self.send_response(200);self.send_header('Content-Type','text/event-stream');self.end_headers()
      for event in [{'type':'response.created','response':{'id':'synthetic','status':'in_progress','output':[]}}, {'type':'response.completed','response':{'id':'synthetic','status':'completed','output':[],'usage':{'input_tokens':1,'output_tokens':0,'total_tokens':1}}}]: self.wfile.write(('data: '+json.dumps(event)+'\n\n').encode())
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler);threading.Thread(target=server.serve_forever,daemon=True).start()
    with tempfile.TemporaryDirectory(prefix='discard-tool-smoke-') as root:
     root=pathlib.Path(root); home=root/'home';home.mkdir();cwd=root/'cwd';cwd.mkdir()
     (home/'config.toml').write_text('[mcp_servers.inherited_synthetic]\nurl=\"http://127.0.0.1:'+str(server.server_port)+'/forbidden\"\n')
     controls['mcp_servers']={'inherited_synthetic':{'enabled':False}}
     actual_home=options.codex_home
     cache=json.loads((actual_home/'models_cache.json').read_text());models=cache['models'];selected=models[0]['slug']
     for m in models: m.update(apply_patch_tool_type=None,shell_type='disabled',experimental_supported_tools=[],multi_agent_version='disabled',node_repl_disabled=True,tool_mode='direct')
     assert cache.get('client_version') == '0.166.0'
     catalog=root/'models.json';catalog.write_text(json.dumps({'models':models}))
     args=[options.codex,'app-server']
     def toml_value(v):
      if isinstance(v,dict):return '{'+', '.join(json.dumps(k)+' = '+toml_value(x) for k,x in v.items())+'}'
      return json.dumps(v)
     def flatten(v):
      for k,x in v.items():args.extend(['-c',k+'='+toml_value(x)])
     flatten(controls)
     flatten({'model_catalog_json':str(catalog),'model_provider':'discard_smoke','model_providers':{'discard_smoke':{'name':'Discard synthetic test','base_url':f'http://127.0.0.1:{server.server_port}/v1','wire_api':'responses','requires_openai_auth':False}}})
     args+=['--listen','stdio://'];env=os.environ.copy();env['CODEX_HOME']=str(home);env.pop('OPENAI_API_KEY',None);env.pop('CODEX_API_KEY',None)
     process=subprocess.Popen(args,cwd=cwd,env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,start_new_session=True)
     def send(obj):process.stdin.write(json.dumps(obj)+'\n');process.stdin.flush()
     def response(i):
      while True:
       line=process.stdout.readline()
       if not line: raise Exception('app-server closed')
       obj=json.loads(line)
       if obj.get('id')==i:
        if 'error' in obj: raise Exception(str(obj['error']))
        return obj['result']
     try:
      send({'id':1,'method':'initialize','params':{'clientInfo':{'name':'discard_smoke','version':'1'}}});response(1);send({'method':'initialized'})
      send({'id':2,'method':'config/read','params':{'cwd':str(cwd),'includeLayers':True}});reported=response(2);effective=reported['config']; effective['tools']={};
      for layer in reversed(reported.get('layers',[])):
       if layer.get('disabledReason') is None:effective['tools'].update(layer['config'].get('tools',{}))
      require_controls(effective,controls)
      send({'id':3,'method':'thread/start','params':{'cwd':str(cwd),'model':selected,'allowProviderModelFallback':False,'approvalPolicy':'never','sandbox':'read-only','ephemeral':True,'config':{'mcp_servers':{'inherited_synthetic':{'enabled':False},'discard-memory':{'url':f'http://127.0.0.1:{server.server_port}/mcp','enabled':True,'omit_tools_from':['deferred','code_mode'],'enabled_tools':['memory_search','memory_list','memory_read','memory_save','memory_edit','memory_forget']}}}}})
      thread=response(3)['thread']['id'];send({'id':4,'method':'turn/start','params':{'threadId':thread,'input':[{'type':'text','text':'Synthetic test. Return an empty answer.'}]}});response(4)
      record=captured.get(timeout=25)
      names=[]
      for t in record['tools'] or []:
       if t.get('type')=='namespace': names.extend(t.get('name','')+'.'+f.get('name','') for f in t.get('tools',[]))
       else:names.append(t.get('name',t.get('type')))
      print('MODEL_TOOL_SURFACE',json.dumps(names));
      allowed={'functions.list_mcp_resources','functions.list_mcp_resource_templates','functions.read_mcp_resource'}|{'mcp__discard_memory.'+n for n in ['memory_search','memory_list','memory_read','memory_save','memory_edit','memory_forget']}
      assert set(names)==allowed,(names,allowed)
      assert not unexpected_mcp,unexpected_mcp
     finally:
      import signal
      os.killpg(process.pid,signal.SIGKILL);process.wait(); process.stderr.read()
    server.shutdown()


if __name__ == "__main__":
    main()
