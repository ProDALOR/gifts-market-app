#!/usr/bin/env python3
"""Disposable current-hub integration; never opens account exports or contacts markets."""
import argparse, json, os, socket, subprocess, tempfile, time, urllib.request, urllib.error, uuid
from decimal import Decimal
from pathlib import Path

parser=argparse.ArgumentParser()
parser.add_argument('--hub-bin',type=Path,required=True)
parser.add_argument('--adapter-manifest',type=Path,required=True)
parser.add_argument('--app-bin',type=Path)
parser.add_argument('--keep',type=Path,help='Keep local preview processes and logs at this ignored path')
args=parser.parse_args();root=Path(__file__).resolve().parent.parent
hub=args.hub_bin.resolve();app=(args.app_bin or root/'target/debug/market-app').resolve();adapter=args.adapter_manifest.resolve().parent
assert hub.is_file() and app.is_file(),'Build the hub and public app binaries first'
workspace=Path(tempfile.mkdtemp(prefix='market-public-')) if not args.keep else args.keep.resolve()
workspace.mkdir(parents=True,exist_ok=True)
def port():
    with socket.socket() as s:
        s.bind(('127.0.0.1',0));return s.getsockname()[1]
grpc,admin,http=port(),port(),port();core=f'http://127.0.0.1:{grpc}';base=f'http://127.0.0.1:{http}'
key=subprocess.check_output([str(hub),'key'],text=True).strip()
env=dict(os.environ,MARKET_SIGNING_KEY=key)
token=subprocess.check_output([str(hub),'token','--id','public-app-test'],env=env,text=True).strip()
processes=[];example_dir=adapter/'crates/market-adapter/examples';created=not example_dir.exists();example_dir.mkdir(exist_ok=True)
name='public_app_fixture_'+uuid.uuid4().hex[:8];example=example_dir/(name+'.rs');example.write_bytes((root/'tests/fixtures/seed.rs').read_bytes())
def start(name,command,environment,cwd):
    stream=(workspace/(name+'.log')).open('ab');p=subprocess.Popen(command,env=environment,cwd=cwd,stdout=stream,stderr=stream,start_new_session=True);processes.append(p);return p
def wait(url):
    for _ in range(150):
        try:
            with urllib.request.urlopen(url,timeout=1):return
        except Exception:time.sleep(.1)
    raise RuntimeError('Local service did not become ready; inspect '+str(workspace))
def call(path,body=None,expected=200):
    data=None if body is None else json.dumps(body).encode();r=urllib.request.Request(base+path,data=data,headers={'Content-Type':'application/json'} if data else {})
    try:
        with urllib.request.urlopen(r,timeout=12) as response:status=response.status;raw=response.read()
    except urllib.error.HTTPError as error:status=error.code;raw=error.read()
    assert status==expected,(path,status,raw[:300])
    assert token.encode() not in raw,'A service credential reached a public response'
    return json.loads(raw) if raw else None
core_command=[str(hub),'--data-dir',str(workspace/'hub'),'--grpc-addr',f'127.0.0.1:{grpc}','--http-addr',f'127.0.0.1:{admin}']
try:
    with (workspace/'fixture-build.log').open('w') as log:
        subprocess.run(['cargo','build','--locked','-p','market-adapter','--example',name],cwd=adapter,stdout=log,stderr=subprocess.STDOUT,check=True)
    fixture_bin=adapter/'target/debug/examples'/name
    def fixture(action):
        with (workspace/'fixture.log').open('a') as log:
            subprocess.run([str(fixture_bin)],env=dict(os.environ,MARKET_NODE_TOKEN=token,MARKET_FIXTURE_CORE=core,MARKET_FIXTURE_ACTION=action),stdout=log,stderr=log,check=True)
    server=start('hub',core_command,env,root);wait(f'http://127.0.0.1:{admin}/healthz');fixture('seed')
    app_command=[str(app),'--bind',f'127.0.0.1:{http}','--core',core,'--web-dir',str(root/'web/dist'),'--data-dir',str(workspace/'replica')]
    gateway=start('app',app_command,dict(os.environ,MARKET_NODE_TOKEN=token),root);wait(base+'/readyz')
    query={'scope':{'item_type_id':'tg.gift.123','currency_id':'GRAM','modes':['ask']},'limit':1}
    page=call('/orders/query',query);assert page['data'][0]['price']=='1.25';assert page['data'][0]['book']['venue_id']=='partner';assert 'T' in page['data'][0]['observed_at']
    stale=page['next_page_token'];second=call('/orders/query',dict(query,page_token=page['next_page_token']));assert second['data'][0]['price']=='2.25';assert second['cursor']==page['cursor']
    q=dict(query,limit=200);result=call('/orders/query',q);prices=[Decimal(v['price']) for v in result['data']];assert len(prices)==30 and prices==sorted(prices)
    descending=call('/orders/query',dict(q,sort='price_desc'));assert [Decimal(v['price']) for v in descending['data']]==sorted(prices,reverse=True)
    bounded=call('/orders/query',dict(q,scope=dict(q['scope'],price={'min':'1.5','max':'3'})));assert [v['price'] for v in bounded['data']]==['2.25']
    stars=call('/orders/query',dict(q,scope=dict(q['scope'],currency_id='XTR')));assert stars['data'][0]['price']=='1250'
    executions=call('/executions/query',{'scope':query['scope']});assert len(executions['data'])==1 and executions['data'][0]['mode']=='executed'
    for old in ['/items','/trades','/status','/market.MarketWrite/Publish']:call(old,expected=404)
    call('/orders/query',dict(query,limit=201),expected=400)
    control=workspace/'control';control.mkdir(exist_ok=True);shots=workspace/'screenshots';shots.mkdir(exist_ok=True)
    for f in control.glob('*'):f.unlink()
    browser=start('browser',['node','scripts/smoke.mjs'],dict(os.environ,MARKET_APP_TEST_URL=base,MARKET_APP_TEST_CONTROL=str(control),MARKET_APP_TEST_SCREENSHOTS=str(shots)),root/'web')
    deadline=time.monotonic()+40
    while not (control/'update.ready').exists():
        if browser.poll() is not None:raise RuntimeError('Browser check stopped; inspect '+str(workspace/'browser.log'))
        assert time.monotonic()<deadline,'Browser fixture timed out';time.sleep(.1)
    fixture('update');(control/'update.done').write_text('done');assert browser.wait(timeout=35)==0,'Browser check failed; inspect '+str(workspace/'browser.log')
    def eventually(check):
        deadline=time.monotonic()+15
        while not check():
            assert time.monotonic()<deadline,'Replica convergence timed out'
            time.sleep(.05)
    # All public queries remain available with the hub offline, including after an app restart.
    server.kill();server.wait()
    eventually(lambda:call('/sync')['state']=='stale')
    assert call('/orders/query',query)['data'][0]['price']=='0.500000001'
    assert len(call('/executions/query',{'scope':query['scope']})['data'])==1
    # A frozen local view survives a hub failure.
    assert call('/orders/query',dict(query,page_token=stale))['data'][0]['price']=='2.25'
    gateway.kill();gateway.wait()
    gateway=start('app',app_command,dict(os.environ,MARKET_NODE_TOKEN=token),root);wait(base+'/readyz')
    assert call('/orders/query',query)['data'][0]['price']=='0.500000001'
    assert len(call('/executions/query',{'scope':query['scope']})['data'])==1
    assert len(call('/catalog/query',{'kind':'item_type'})['data'])==2
    call('/orders/query',dict(query,page_token=stale),expected=409)
    # Hub restart invalidates the Watch cursor: rebuild and swap the replica atomically.
    server=start('hub',core_command,env,root);wait(f'http://127.0.0.1:{admin}/healthz');fixture('seed-live')
    eventually(lambda:call('/sync')['state']=='live' and len(call('/orders/query',q)['data'])==30 and call('/orders/query',query)['data'][0]['price']=='1.25')
    assert len(call('/executions/query',{'scope':query['scope']})['data'])==1
    # A SNAPSHOT continuation whose retained root expired is still a valid LIVE boundary.
    probe=r'''
const q=JSON.parse(process.env.PUBLIC_QUERY),base=process.env.PUBLIC_URL;
const socket=new WebSocket(base.replace('http:','ws:')+'/ws');
const page=await new Promise((resolve,reject)=>{const timeout=setTimeout(()=>reject(Error('live boundary timed out')),5000);socket.onopen=()=>socket.send(JSON.stringify({type:'subscribe',id:'token',query:q}));socket.onmessage=e=>{const frame=JSON.parse(e.data);if(frame.type==='error')reject(Error(frame.error.message));if(frame.type==='page'){clearTimeout(timeout);resolve(frame.page);}};socket.onerror=reject;});
if(page.data[0].price!=='2.25')throw Error('Snapshot anchor was not reused in live mode');
socket.close();const response=await fetch(base+'/orders/query',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({...q,page_token:page.next_page_token})});const next=await response.json();if(!response.ok||next.data[0].price!=='3.25')throw Error('Live anchor was not reused in snapshot mode');
'''
    subprocess.run(['node','--input-type=module','-e',probe],env=dict(os.environ,PUBLIC_QUERY=json.dumps(dict(query,page_token=stale)),PUBLIC_URL=base),check=True,timeout=10)
    # Execution transitions and late backfills are indexed locally and remain durable offline.
    fixture('execute')
    eventually(lambda:len(call('/orders/query',q)['data'])==29 and len(call('/executions/query',{'scope':query['scope']})['data'])==2)
    fixture('backfill')
    eventually(lambda:len(call('/executions/query',{'scope':query['scope']})['data'])==3)
    history=call('/executions/query',{'scope':query['scope']})['data']
    assert history[0]['id']=='old-execution'
    server.kill();server.wait();eventually(lambda:call('/sync')['state']=='stale')
    gateway.kill();gateway.wait();gateway=start('app',app_command,dict(os.environ,MARKET_NODE_TOKEN=token),root);wait(base+'/readyz')
    assert len(call('/executions/query',{'scope':query['scope']})['data'])==3
    assert len(call('/orders/query',q)['data'])==29
    # Leave the optional preview connected after testing persisted, disconnected reads.
    server=start('hub',core_command,env,root);wait(f'http://127.0.0.1:{admin}/healthz')
    fixture('seed-recover')
    eventually(lambda:call('/sync')['state']=='live' and len(call('/orders/query',q)['data'])==29)
    assert len(call('/executions/query',{'scope':query['scope']})['data'])==3
    (workspace/'preview.json').write_text(json.dumps({'url':base,'hub_http':f'http://127.0.0.1:{admin}','grpc':core,'pids':[server.pid,gateway.pid]}))
    print('Verified local REST/WebSocket reads, snapshot catch-up, price ranking, filters, shared tokens, persisted offline app restart, hub restart rebuild, terminal transitions and late execution backfills.')
    print('Preview: '+base if args.keep else 'Disposable services stopped.')
except BaseException:
    for p in processes:
        if p.poll() is None:p.kill()
    raise
finally:
    example.unlink(missing_ok=True)
    if created and not list(example_dir.iterdir()):example_dir.rmdir()
    if not args.keep:
        for p in reversed(processes):
            if p.poll() is None:p.terminate()
        for p in processes:
            try:p.wait(timeout=6)
            except subprocess.TimeoutExpired:p.kill();p.wait()
