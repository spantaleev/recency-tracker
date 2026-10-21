const assert = require('node:assert/strict');
const { spawn, execFile } = require('node:child_process');
const { promisify } = require('node:util');
const { once } = require('node:events');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const net = require('node:net');
const crypto = require('node:crypto');
const vm = require('node:vm');
const { setTimeout: delay } = require('node:timers/promises');
const test = require('node:test');
const { WebSocket } = require('ws');
const run = promisify(execFile);
const root = path.resolve(__dirname, '../..');
const implementation = process.env.RECENCY_TEST_IMPLEMENTATION || 'rust';
const nodeRoot = path.resolve(process.env.RECENCY_TEST_NODE_ROOT || path.join(root, 'node-reference'));
const secret = 'test-secret/with&punctuation\\and$quotes"';
const timeout = () => ({ signal: AbortSignal.timeout(5000) });
async function until(check, milliseconds = 5000) {
 const end = Date.now() + milliseconds;
 while (!(await check())) { assert(Date.now() < end, 'Timed out waiting for expected behavior'); await delay(10); }
}
async function freePort() { const s=net.createServer(); s.listen(0,'127.0.0.1');await once(s,'listening');const p=s.address().port;await new Promise(r=>s.close(r));return p; }
async function fixture(t, {limits, filesystem=true, listenerHost='127.0.0.1'} = {}) {
 const directory=await fs.mkdtemp(path.join(os.tmpdir(),'recency-contract-'));
 const storage=path.join(directory,'data');await fs.mkdir(storage);
 const config={publisher:{ip:listenerHost,port:await freePort(),secret:'overridden'},subscriber:{ip:listenerHost,port:await freePort()},persister:{type:filesystem?'filesystem':'none',memory:{maxItems:2},filesystem:{storagePath:storage}},...(limits?{limits}:{})};
 const configFile=path.join(directory,'config.json');await fs.writeFile(configFile,JSON.stringify(config));
 let child; const sockets=[];
 const pub=`http://${listenerHost}:${config.publisher.port}`,sub=`http://${listenerHost}:${config.subscriber.port}`;
 async function start(selected=implementation) {
  const command=selected==='node'?process.execPath:path.join(root,'rust/target/debug/recency-tracker');
  child=spawn(command,selected==='node'?['bin/server']:[],{cwd:selected==='node'?nodeRoot:root,env:{...process.env,RECENCY_TRACKER_CONFIG:configFile,RECENCY_TRACKER_PUBLISHER_SECRET:secret},stdio:['ignore','ignore','pipe']});
  let logs='';child.stderr.on('data',d=>{logs=(logs+d).slice(-4000);});
  await until(async()=>{assert.equal(child.exitCode,null,logs);try{return (await fetch(pub+'/ping',timeout())).ok;}catch{return false;}});
 }
 async function stop() {if(child&&child.exitCode===null){const exited=once(child,'exit',timeout());child.kill('SIGTERM');const[code,signal]=await exited;assert.equal(signal,null);assert.equal(code,0);}child=null;}
 t.after(async()=>{for(const s of sockets)s.terminate();try{await stop();}finally{await fs.rm(directory,{recursive:true,force:true});}});
 await start();
 async function connect(options) {const socket=new WebSocket(sub.replace('http:','ws:')+'/subscribe',options);sockets.push(socket);socket.on('error',()=>{});const messages=[];socket.on('message',d=>messages.push(JSON.parse(d)));await once(socket,'open',timeout());return {socket,messages,send:m=>socket.send(JSON.stringify(m))};}
 async function publish(resource,version,data={}) {const r=await fetch(pub+'/publish',{method:'POST',body:new URLSearchParams({secret,resource,version,data:JSON.stringify(data)}),...timeout()});assert.equal(r.status,200);assert.deepEqual(await r.json(),{ok:true});}
 return {pub,sub,connect,publish,start,stop,storage,configFile,config};
}

test('health, client asset, authentication and ordinary publisher errors',async t=>{
 const f=await fixture(t);
 assert.equal(await(await fetch(f.sub+'/ping')).text(),'subscriber server up');
 assert.equal(await(await fetch(f.pub+'/ping')).text(),'publisher server up');
 assert.equal(await(await fetch(f.sub+'/recency-tracker.js')).text(),await fs.readFile(path.join(root,'resources/static/recency-tracker.js'),'utf8'));
 assert.equal((await fetch(f.sub+'/subscribe/info')).status,404);
 for(const [fields,error] of [[{},'bad request'],[{secret:'wrong',resource:'/example',version:'1'},'bad secret'],[{secret,resource:'/example',version:'1',data:'{'},'bad data - invalid json']]) {
  const r=await fetch(f.pub+'/publish',{method:'POST',body:new URLSearchParams(fields),...timeout()});assert.equal(r.status,200);assert.deepEqual(await r.json(),{ok:false,error});
 }
 const body=new URLSearchParams({secret,resource:'/example',version:'1'}).toString()+'&secret='+encodeURIComponent(secret);
 const duplicate=await fetch(f.pub+'/publish',{method:'POST',headers:{'content-type':'application/x-www-form-urlencoded'},body,...timeout()});assert.deepEqual(await duplicate.json(),{ok:false,error:'bad secret'});
});

test('listener hostnames and trailing slashes retain the v2 HTTP behavior',async t=>{
 const f=await fixture(t,{listenerHost:'localhost'});
 assert.equal(await(await fetch(f.pub+'/ping/')).text(),'publisher server up');
 assert.equal(await(await fetch(f.sub+'/ping/')).text(),'subscriber server up');
 assert.equal(await(await fetch(f.pub+'/PING/')).text(),'publisher server up');
 assert.equal(await(await fetch(f.sub+'/PING/')).text(),'subscriber server up');
 const response=await fetch(f.pub+'/publish/',{method:'POST',body:new URLSearchParams({secret,resource:'/example/slash',version:'1'}),...timeout()});
 assert.equal(response.status,200);
 assert.deepEqual(await response.json(),{ok:true});
 const uppercase=await fetch(f.pub+'/PUBLISH/',{method:'POST',body:new URLSearchParams({secret,resource:'/example/uppercase',version:'1'}),...timeout()});
 assert.equal(uppercase.status,200);
 assert.deepEqual(await uppercase.json(),{ok:true});
 const c=await f.connect();c.send({type:'subscribe',resource:'/example/slash',version:'0'});
 await until(()=>c.messages.length===1);assert.equal(c.messages[0].version,'1');
 c.send({type:'subscribe',resource:'/example/uppercase',version:'0'});
 await until(()=>c.messages.length===2);assert.equal(c.messages[1].resource,'/example/uppercase');
});

test('version ordering, numeric coercion, duplicate subscriptions and unsubscribe',async t=>{
 const f=await fixture(t),c=await f.connect();const resource='/example/😀';
 c.send({type:'subscribe',resource,version:0.0});await f.publish(resource,'2',{actor:'example-user'});await until(()=>c.messages.length===1);
 assert.deepEqual(c.messages[0],{type:'update',resource,version:'2',data:{actor:'example-user'}});
 for(const version of ['10','9','0001','0000','😀','\ue000']) await f.publish(resource,version);
 await until(()=>c.messages.length===6);assert.deepEqual(c.messages.map(m=>m.version),['2','10','9','0001','😀','\ue000']);
 c.send({type:'subscribe',resource,version:'0'});await delay(80);assert.equal(c.messages.length,6);
 c.send({type:'unsubscribe',resource});await delay(80);await f.publish(resource,'999999');await delay(80);assert.equal(c.messages.length,6);
});

for(const [name,payload,code] of [['invalid JSON','{',1007],['binary',Buffer.from('{}'),1003],['oversized','x'.repeat(65537),1009]]) {
 test(`WebSocket ${name} close code`,async t=>{const f=await fixture(t),c=await f.connect();const closed=once(c.socket,'close',timeout());c.socket.send(payload);assert.equal((await closed)[0],code);});
}

test('invalid commands are ignored without breaking a valid subscription',async t=>{
 const f=await fixture(t),c=await f.connect();for(const message of [null,[],{},1,{type:'unknown',resource:'/example',version:'0'},{type:'subscribe',resource:'constructor',version:'0'},{type:'subscribe',resource:'/example',version:{}}])c.send(message);
 c.send({type:'subscribe',resource:'/example',version:'0'});await f.publish('/example','1');await until(()=>c.messages.length===1);
});

test('persisted JSON survives restart with a live socket and cache eviction',async t=>{
 const f=await fixture(t);const resource='/example/persisted';const c=await f.connect();c.send({type:'subscribe',resource,version:'0'});
 await f.publish(resource,'12',{list:[null,false,1,'β']});await until(()=>c.messages.length===1);
 for(let i=0;i<5;i++)await f.publish(`/other/${i}`,'1');
 const filename=path.join(f.storage,crypto.createHash('sha256').update(resource).digest('hex')+'.json');
 await until(async()=>{try{return JSON.parse(await fs.readFile(filename,'utf8')).version==='12';}catch{return false;}});
 await f.stop();await f.start();const reader=await f.connect();reader.send({type:'subscribe',resource,version:0});await until(()=>reader.messages.length===1);assert.deepEqual(reader.messages[0].data,{list:[null,false,1,'β']});
});

test('unchanged browser client reconnects with its last seen version',async t=>{
 const f=await fixture(t);let reconnect;
 const browser=vm.createContext({WebSocket:global.WebSocket,URL});browser.window=browser;browser.location={href:'http://example.test/page'};browser.setTimeout=cb=>{reconnect=cb;return 1;};browser.clearTimeout=()=>{reconnect=null;};
 vm.runInContext(await fs.readFile(path.join(root,'resources/static/recency-tracker.js'),'utf8'),browser);
 const client=new browser.RecencyTracker(f.sub+'/subscribe');t.after(()=>client.unsubscribe('/example/browser'));const updates=[];client.subscribe('/example/browser',0,(v,d)=>updates.push({v,d}));
 await f.publish('/example/browser','1',{example:true});await until(()=>updates.length===1);client._sock.close();await until(()=>!!reconnect);
 await f.publish('/example/browser','2');reconnect();await until(()=>updates.length===2);assert.equal(updates[1].v,'2');client.unsubscribe('/example/browser');
});

test('existing PHP publisher works without modifications', {skip:!process.env.RECENCY_TEST_PHP_SOURCE},async t=>{
 const f=await fixture(t),c=await f.connect();const resource='/example/php';c.send({type:'subscribe',resource,version:0});
 const source=process.env.RECENCY_TEST_PHP_SOURCE;
 const script='require $argv[1]."/Exception.php"; require $argv[1]."/Publisher.php"; $publisher=new \\Devture\\Component\\RecencyTracker\\Publisher($argv[2],$argv[3]); $publisher->publish("/example/php", 42, ["actor"=>"example-user","nested"=>["flag"=>true]]);';
 await run('php',['-r',script,source,f.pub+'/PUBLISH/',secret],{timeout:10000});await until(()=>c.messages.length===1);
 assert.deepEqual(c.messages[0],{type:'update',resource,version:'42',data:{actor:'example-user',nested:{flag:true}}});
});

test('Rust rejects excess parameters and corrupt storage without terminating', {skip:implementation!=='rust'},async t=>{
 const f=await fixture(t);const body=new URLSearchParams({secret,resource:'/example',version:'1'}).toString()+'&'+Array.from({length:1000},(_,i)=>`extra${i}=1`).join('&');
 assert.equal((await fetch(f.pub+'/publish',{method:'POST',headers:{'content-type':'application/x-www-form-urlencoded'},body,...timeout()})).status,413);
 const resource='/example/corrupt';await fs.writeFile(path.join(f.storage,crypto.createHash('sha256').update(resource).digest('hex')+'.json'),'{');
 const c=await f.connect(),closed=once(c.socket,'close',timeout());c.send({type:'subscribe',resource,version:0});assert.equal((await closed)[0],1011);
 await f.publish('/example/healthy','1');assert.equal((await fetch(f.pub+'/ping')).status,200);
});

test('storage write failure is reported and service recovers after repair', {skip:implementation!=='rust'},async t=>{
 const f=await fixture(t);await f.publish('/example/recovery','1');await fs.rename(f.storage,f.storage+'.moved');
 const response=await fetch(f.pub+'/publish',{method:'POST',body:new URLSearchParams({secret,resource:'/example/recovery',version:'2'}),...timeout()});assert.equal(response.status,500);assert.deepEqual(await response.json(),{ok:false,error:'resource storage unavailable'});
 assert.equal((await fetch(f.pub+'/ping')).status,200);await fs.rename(f.storage+'.moved',f.storage);await f.publish('/example/recovery','3');
 const c=await f.connect();c.send({type:'subscribe',resource:'/example/recovery',version:'1'});await until(()=>c.messages.length===1);assert.equal(c.messages[0].version,'3');
});

test('connection and subscription limits close clients explicitly', {skip:implementation!=='rust'},async t=>{
 const f=await fixture(t,{limits:{connections:1,subscriptions_per_connection:1}}),first=await f.connect();
 const second=new WebSocket(f.sub.replace('http:','ws:')+'/subscribe');second.on('error',()=>{});await assert.rejects(once(second,'open',timeout()),/503/);second.terminate();
 first.send({type:'subscribe',resource:'/example/one',version:'0'});const closed=once(first.socket,'close',timeout());first.send({type:'subscribe',resource:'/example/two',version:'0'});assert.equal((await closed)[0],1013);
 const fresh=await f.connect();fresh.send({type:'subscribe',resource:'/example/one',version:'0'});await f.publish('/example/one','1');await until(()=>fresh.messages.length===1);
});

test('browser reconnects after a subscription limit closes its socket', {skip:implementation!=='rust'},async t=>{
 const f=await fixture(t,{limits:{subscriptions_per_connection:2}});let reconnect;
 const browser=vm.createContext({WebSocket:global.WebSocket,URL});browser.window=browser;browser.location={href:'http://example.test/page'};browser.setTimeout=cb=>{reconnect=cb;return 1;};browser.clearTimeout=()=>{reconnect=null;};
 vm.runInContext(await fs.readFile(path.join(root,'resources/static/recency-tracker.js'),'utf8'),browser);
 const client=new browser.RecencyTracker(f.sub+'/subscribe');const resources=['/example/one','/example/two','/example/overflow'];
 t.after(()=>{for(const resource of resources)client.unsubscribe(resource);});
 const updates=[];for(const resource of resources)client.subscribe(resource,'0',(v,d)=>updates.push({resource,v,d}));
 await until(()=>!!reconnect);client.unsubscribe('/example/overflow');reconnect();
 await f.publish('/example/one','1',{recovered:true});await until(()=>updates.length===1);
 assert.equal(updates[0].resource,'/example/one');assert.equal(updates[0].v,'1');
 assert.deepEqual(JSON.parse(JSON.stringify(updates[0].d)),{recovered:true});
});


test('invalid UTF-8 text closes with code 1007',async t=>{
 const f=await fixture(t),c=await f.connect();const closed=once(c.socket,'close',timeout());c.socket.send(Buffer.from([0xff]),{binary:false});assert.equal((await closed)[0],1007);
});

test('real heartbeat drops unresponsive sockets and keeps responsive sockets', {skip:process.env.RECENCY_TEST_HEARTBEAT!=='1'},async t=>{
 const f=await fixture(t,{filesystem:false}),good=await f.connect(),bad=await f.connect({autoPong:false});let pings=0;good.socket.on('ping',()=>pings++);
 await once(bad.socket,'close',{signal:AbortSignal.timeout(66000)});await until(()=>pings>=2);assert.equal(good.socket.readyState,WebSocket.OPEN);
 good.send({type:'subscribe',resource:'/example/heartbeat',version:0});await f.publish('/example/heartbeat','1');await until(()=>good.messages.length===1);
});


test("Node and Rust read each other's persisted records without conversion",async t=>{
 const f=await fixture(t);await f.stop();await f.start('node');await f.publish('/example/migration','1',{before:true});await f.stop();
 await f.start('rust');let c=await f.connect();c.send({type:'subscribe',resource:'/example/migration',version:0});await until(()=>c.messages.length===1);assert.deepEqual(c.messages[0].data,{before:true});
 await f.publish('/example/migration','2',{after:true});await f.stop();await f.start('node');c=await f.connect();c.send({type:'subscribe',resource:'/example/migration',version:0});await until(()=>c.messages.length===1);assert.deepEqual(c.messages[0].data,{after:true});
});

test('invalid upgrade headers do not consume WebSocket admission', {skip:implementation!=='rust'}, async t=>{
 const http=require('node:http');
 const f=await fixture(t,{limits:{connections:1}});
 const valid={Connection:'Upgrade',Upgrade:'websocket','Sec-WebSocket-Version':'13','Sec-WebSocket-Key':'dGhlIHNhbXBsZSBub25jZQ=='};
 for(const change of [
  {'Sec-WebSocket-Key':'not-base64'},
  {'Sec-WebSocket-Key':Buffer.alloc(15).toString('base64')},
  {'Sec-WebSocket-Key':Buffer.alloc(17).toString('base64')},
  {'Sec-WebSocket-Version':'12'},
  {Connection:'keep-alive'},
  {Upgrade:'other-protocol'},
 ]) {
  const status=await new Promise((resolve,reject)=>{
   const req=http.get(f.sub+'/subscribe',{headers:{...valid,...change},agent:false},res=>{res.resume();resolve(res.statusCode);});
   req.on('error',reject);req.setTimeout(3000,()=>req.destroy(Error('HTTP probe timeout')));
  });
  assert.equal(status,400);
 }
 const client=await f.connect();client.send({type:'subscribe',resource:'/example/handshake',version:0});
 await f.publish('/example/handshake','1');await until(()=>client.messages.length===1);
});
