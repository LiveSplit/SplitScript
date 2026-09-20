import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {SplitScriptHost} from './support/splitscript_host.mjs';
import {createMonoPeFixture} from './support/mono_pe_fixture.mjs';
import {createIl2cppPeFixture} from './support/il2cpp_pe_fixture.mjs';
import {writeManagedObjectHeader} from './support/managed_type_fixture.mjs';
const [wasm]=process.argv.slice(2);
const profiles=JSON.parse(await readFile(new URL('./fixtures/mono-pe-profiles.json',import.meta.url)));
const layouts={
    V1:{32:[0x74,0x64,0x24],64:[0xa8,0x94,0x30]},
    V1Cattrs:{32:[0x78,0x68,0x24],64:[0xb0,0x9c,0x30]},
    V2:{32:[0x60,0xa4,0x20],64:[0x98,0x100,0x30]},
    V3:{32:[0x60,0x9c,0x20],64:[0x98,0x100,0x30]},
    il2cpp:{32:[0x40,0xac,0x2c],64:[0x80,0x124,0x58]},
};
const modes=['same','derived','deep derived','wrong class','null header','unreadable header','null vtable class','unreadable vtable class','unreadable parent','hierarchy cycle','hierarchy limit','torn class','cached ancestry','repair ancestry','reattach'];
let cases=0;
for(const family of Object.keys(layouts))for(const width of [32,64])for(const mode of modes){
    const mono=family!=='il2cpp',bytes=width/8;
    if(!mono&&mode.includes('vtable'))continue;
    const f=mono?createMonoPeFixture(profiles.builds.find(p=>p.width===width&&p.version===family)):createIl2cppPeFixture({width,version:[2022,3,0,37029]});
    const {memory}=f;
    const number=(at,size,value)=>{const data=new Uint8Array(size),v=new DataView(data.buffer);if(size===8)v.setBigUint64(0,BigInt(value),true);else if(size===4)v.setUint32(0,Number(value),true);else if(size===2)v.setUint16(0,Number(value),true);else v.setUint8(0,Number(value));data.forEach((b,i)=>memory.set(at+BigInt(i),b));};
    const ptr=(at,value)=>number(at,bytes,value);
    const name=(at,text)=>{const data=new Uint8Array(256);data.set(new TextEncoder().encode(text));data.forEach((b,i)=>memory.set(at+BigInt(i),b));};
    const [fields,count,parent]=layouts[family][width];
    ptr(0x14000n+BigInt(fields),0x50000n);number(0x14000n+BigInt(count),mono?4:2,2);
    ['instance','value'].forEach((text,i)=>{const at=0x50000n+BigInt(i*(width===64?32:mono?16:20));ptr(at+BigInt(mono?bytes:0),0x60000n+BigInt(i*256));name(0x60000n+BigInt(i*256),text);number(at+BigInt(width===64?0x18:0xc),4,0x10);});
    const object=0x70000n,derived=0x30000n,other=0x31000n;
    ptr((mono?0x18000n:0x16000n)+0x10n,object);
    const original=writeManagedObjectHeader(f,{mono,ptr},object);
    number(object+0x10n,4,7);ptr(derived+BigInt(parent),0x14000n);ptr(other+BigInt(parent),0);
    let armed=false,torn=false;const reads=[],read=f.process.read;
    f.process.read=request=>{const at=BigInt.asUintN(64,request.address);reads.push(at);const result=read({...request,address:at});if(armed&&at===object+0x10n){torn=!torn;writeManagedObjectHeader(f,{mono,ptr},object,torn?derived:0x14000n);}return result;};
    const host=await SplitScriptHost.instantiate(wasm);host.addProcess('game.exe',f.process);host.start();
    const label=`${family}/${width}/${mode}`;
    host.updateUntil(()=>host.variables.get('value')==='7',label);host.update(2);reads.length=0;number(object+0x10n,4,9);
    if(['derived','cached ancestry','repair ancestry','reattach','unreadable parent','hierarchy cycle','hierarchy limit','deep derived'].includes(mode))writeManagedObjectHeader(f,{mono,ptr},object,derived);
    if(mode==='wrong class')writeManagedObjectHeader(f,{mono,ptr},object,other);
    if(mode==='null header')ptr(object,0);
    if(mode==='unreadable header')memory.delete(object);
    if(mode==='null vtable class')ptr(original.vtable,0);
    if(mode==='unreadable vtable class')memory.delete(original.vtable);
    if(mode==='unreadable parent')memory.delete(derived+BigInt(parent));
    if(mode==='repair ancestry')ptr(derived+BigInt(parent),0);
    if(mode==='hierarchy cycle')ptr(derived+BigInt(parent),derived);
    if(mode==='deep derived'||mode==='hierarchy limit'){
        const depth=mode==='deep derived'?128:129;
        for(let i=0;i<depth;i++)ptr(derived+BigInt(i*0x1000+parent),i===depth-1?0x14000n:derived+BigInt((i+1)*0x1000));
    }
    if(mode==='torn class')armed=true;
    host.update();
    const success=['same','derived','deep derived','cached ancestry','reattach'].includes(mode);
    assert.equal(host.variables.get('result')==='ok',success,`${label}: ${host.variables.get('result')}`);
    assert.equal(host.variables.get('value'),success?'9':'7',`${label}: snapshot transaction`);assert.equal(host.variables.get('old'),'7',label);
    // Empty snapshots still validate identity, even without any field reads.
    if(mode!=='torn class')assert.equal(host.variables.get('empty')==='ok',success,`${label}: empty snapshot`);
    if(!success&&mode!=='torn class')assert(!reads.includes(object+0x10n),`${label}: invalid class read payload`);
    if(mode==='cached ancestry'){
        memory.delete(derived+BigInt(parent));reads.length=0;host.update();assert.equal(host.variables.get('result'),'ok',label);assert(!reads.includes(derived+BigInt(parent)),`${label}: reread cached ancestry`);
    }
    if(mode==='reattach'){
        host.setProcessOpen('game.exe',false);host.update(3);ptr(derived+BigInt(parent),0);reads.length=0;host.setProcessOpen('game.exe',true);
        host.updateUntil(()=>reads.includes(derived+BigInt(parent)),`${label}: fresh attachment cache`);
        assert(!reads.includes(object+0x10n),`${label}: stale proof read payload`);
        ptr(derived+BigInt(parent),0x14000n);number(object+0x10n,4,11);
        host.updateUntil(()=>host.variables.get('value')==='11',`${label}: repaired reattachment`);
    }
    if(!success){
        armed=false;writeManagedObjectHeader(f,{mono,ptr},object,mode==='repair ancestry'?derived:0x14000n);ptr(derived+BigInt(parent),0x14000n);
        host.updateUntil(()=>host.variables.get('result')==='ok'&&host.variables.get('value')==='9',`${label}: repair`);
    }
    assert(reads.length<1200,`${label}: unbounded hierarchy traversal`);cases++;
}
console.log(JSON.stringify({managedSnapshotIdentityCases:cases}));
