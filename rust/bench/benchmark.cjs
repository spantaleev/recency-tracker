// Local-only evaluation. All Docker mutations use this invocation's unique names.
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const assert = require('node:assert/strict');
const { once } = require('node:events');
const { setTimeout: delay } = require('node:timers/promises');
const { WebSocket } = require('ws');
const docker = (...args) => execFileSync('docker', args, { encoding:'utf8', timeout:60000 }).trim();
const variants = [
 ['node', process.env.RECENCY_BENCH_NODE_IMAGE || 'recency-tracker:node-reference'],
 ['rust', process.env.RECENCY_BENCH_RUST_IMAGE || 'recency-tracker:rust-candidate'],
];
const output = process.env.RECENCY_BENCH_OUTPUT || path.join(os.tmpdir(), 'recency-benchmark.json');
const results=[];
let container;
let directory;
let sockets=[];
function sample(pid) {
 const group=fs.readFileSync(`/proc/${pid}/cgroup`,'utf8').trim().split(':').at(-1);
 const root=path.join('/sys/fs/cgroup',group);
 const stat=Object.fromEntries(fs.readFileSync(root+'/memory.stat','utf8').trim().split('\n').map(s=>s.split(' ').map((v,i)=>i?Number(v):v)));
 const current=Number(fs.readFileSync(root+'/memory.current','utf8'));
 const rss=Number(fs.readFileSync(`/proc/${pid}/status`,'utf8').match(/^VmRSS:\s+(\d+)/m)[1])*1024;
 return { current, working:current-stat.inactive_file, rss, anon:stat.anon, kernel:stat.kernel, sock:stat.sock, file:stat.file };
}
async function samples(pid) {
 const a=[];
 for(let i=0;i<5;i++){a.push(sample(pid)); await delay(200);}
 return Object.fromEntries(Object.keys(a[0]).map(k=>[k,a.map(v=>v[k]).sort((a,b)=>a-b)[2]]));
}
async function wait(check,ms=15000) {
 const until=Date.now()+ms;
 while(!check()) {if(Date.now()>until)throw Error('timeout'); await delay(5);}
}
async function main() {
 for(let repeat=1;repeat<=3;repeat++) {
  // Reverse on alternating runs to reduce systematic run-order bias.
  for(const [variant,image,buffer] of (repeat%2?variants:[...variants].reverse())) {
   directory=fs.mkdtempSync(path.join(os.tmpdir(),'recency-eval-bench-'));
   fs.chmodSync(directory,0o755); fs.mkdirSync(directory+'/data', {mode:0o777}); fs.chmodSync(directory+'/data',0o777);
   const config=JSON.parse(fs.readFileSync('config/container.json.dist')); config.publisher.secret='evaluation';
   fs.writeFileSync(directory+'/config.json',JSON.stringify(config),{mode:0o444});
   container=`recency-eval-${process.pid}-${repeat}-${variant}`;
   docker('run','-d','--name',container,'--read-only','--cpus=1','--memory=1g','--cap-drop=ALL','--security-opt=no-new-privileges','--log-driver=none',
    '--mount',`type=bind,src=${directory}/config.json,dst=/config/config.json,readonly`,
    '--mount',`type=bind,src=${directory}/data,dst=/data`,
    '-p','127.0.0.1::9998','-p','127.0.0.1::9999',...(buffer?['-e',`EVAL_WS_READ_BUFFER=${buffer}`]:[]),image);
   const info=JSON.parse(docker('inspect',container))[0]; const pid=info.State.Pid;
   const pub='http://127.0.0.1:'+info.NetworkSettings.Ports['9998/tcp'][0].HostPort;
   const sub='ws://127.0.0.1:'+info.NetworkSettings.Ports['9999/tcp'][0].HostPort+'/subscribe';
   for(let i=0;i<100;i++) {try {if((await fetch(pub+'/ping')).ok)break;}catch{} await delay(50);}
   await delay(2000);
   const metrics=[]; metrics.push({connections:0,phase:'idle',...await samples(pid)});
   let received=0; let socketErrors=0;
   for(const target of [100,1000,5000]) {
    while(sockets.length<target) {
     await Promise.all(Array.from({length:Math.min(50,target-sockets.length)},async()=>{
      const socket=new WebSocket(sub); sockets.push(socket);
      socket.on('error',()=>socketErrors++);
      socket.on('message',data=>{const m=JSON.parse(data); assert.equal(m.type,'update'); received++;});
      await once(socket,'open',{signal:AbortSignal.timeout(10000)});
      socket.send(JSON.stringify({type:'subscribe',resource:'/bench/shared',version:'0'}));
     }));
    }
    await delay(1000);
    metrics.push({connections:target,phase:'subscribed',...await samples(pid)});
    if(target===1000) {
     const latencies=[]; let expected=0;
     for(let version=1;version<=20;version++) {
      const start=performance.now();
      const response=await fetch(pub+'/publish',{method:'POST',body:new URLSearchParams({secret:'evaluation',resource:'/bench/shared',version:String(version),data:JSON.stringify({payload:'x'.repeat(256)})}),signal:AbortSignal.timeout(15000)});
      assert.deepEqual(await response.json(),{ok:true}); expected+=target;
      await wait(()=>received>=expected);
      latencies.push(performance.now()-start);
     }
     assert.equal(received,20000);
     metrics.push({connections:target,phase:'after-20-fanouts',...await samples(pid)});
     latencies.sort((a,b)=>a-b);
     metrics.push({phase:'fanout-latency-ms',connections:target,p50:latencies[9],p95:latencies[18],max:latencies[19],updates:received});
    }
   }
   assert.equal(socketErrors,0); assert(sockets.every(s=>s.readyState===WebSocket.OPEN));
   for(const socket of sockets)socket.terminate(); sockets=[];
   await delay(1000); metrics.push({connections:0,phase:'after-disconnect',...await samples(pid)});
   docker('stop','-t','5',container); assert.equal(docker('inspect','--format','{{.State.ExitCode}}',container),'0');
   docker('rm',container); container=null; fs.rmSync(directory,{recursive:true,force:true}); directory=null;
   const result={repeat,variant,image,metrics}; results.push(result);
   fs.writeFileSync(output,JSON.stringify(results,null,2)+'\n');
   console.log(JSON.stringify(result));
  }
 }
}
main().catch(e=>{console.error(e);process.exitCode=1;}).finally(()=>{
 for(const socket of sockets)socket.terminate();
 if(container)try{docker('rm','-f',container);}catch{}
 if(directory)fs.rmSync(directory,{recursive:true,force:true});
});
