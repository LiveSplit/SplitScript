import {writeVectorElementType, writeManagedArrayType} from './support/managed_type_fixture.mjs';
import assert from 'node:assert/strict';
import {SplitScriptHost} from './support/splitscript_host.mjs';
import {createKeyedCollectionFixture} from './support/keyed_collection_fixture.mjs';

const wasm = process.argv[2];
const layouts = {
    V1Cattrs: {32: [0x34,0x38,0x78,0x68,null],64: [0x50,0x58,0xb0,0x9c,null]},
    V2: {32: [0x2c,0x30,0x60,0xa4,0x1e],64: [0x48,0x50,0x98,0x100,0x2a]},
    V3: {32: [0x2c,0x30,0x60,0x9c,0xf],64: [0x48,0x50,0x98,0x100,0x1b]},
    il2cpp: {32: [8,0xc,0x40,0xac,null],64: [0x10,0x18,0x80,0x124,null]},
};
let cases = 0, maximumFieldReads = 0;
for (const family of Object.keys(layouts)) for (const width of [32,64]) for (const kind of [0,1,2]) {
    const f = createKeyedCollectionFixture({family,width,dictionary:kind !== 2});
    const {memory,number,ptr,bytes,owner,root,object,outer} = f;
    const mono = family !== 'il2cpp', wide = width === 64;
    const [nameOffset,namespaceOffset,fieldsOffset,countOffset,kindOffset] = layouts[family][width];
    const fieldStride = wide ? 32 : mono ? 16 : 20;
    const text = (at, value) => {
        const data = new Uint8Array(256); data.set(new TextEncoder().encode(value));
        data.forEach((byte,i)=>memory.set(at+BigInt(i),byte));
    };
    // The public schema binds three independent roots; only the selected root is read.
    ptr(0x14000n+BigInt(fieldsOffset),0x58000n);
    number(0x14000n+BigInt(countOffset),mono?4:2,3);
    ['lists','maps','sets'].forEach((name,i)=>{
        const at=0x58000n+BigInt(i*fieldStride), label=0x59000n+BigInt(i*256);
        ptr(at+BigInt(mono?bytes:0),label); text(label,name);
        number(at+BigInt(wide?0x18:0xc),4,0x10+i*bytes);
        ptr((mono?0x18000n:0x16000n)+BigInt(0x10+i*bytes),0x80000n);
    });
    number(0x60080n,1,kind);
    if (kind===0) {
        text(0x40200n,'List`1');
        writeVectorElementType({mono, width, family, ptr, number}, 0x50000n, 0x56000n, 0x0e);
        text(0x46000n,'_items'); text(0x46100n,'_size');
        ptr(0x37000n+BigInt(mono?bytes:0),0x46000n);
        ptr(0x37000n+BigInt(fieldStride+(mono?bytes:0)),0x46100n);
        ptr(0x37000n+BigInt(fieldStride+(mono?0:bytes)),0x50200n);
    }
    // A complete 4096-field class is legal, but four cold sibling layouts must
    // share the 16384-work root limit. Most names are unrelated and still need
    // scanning to prove there are no duplicate required fields.
    const fieldArray=0x100000n, count=4096, selected=kind===0?2:outer.length;
    text(0x47000n,'unrelated');
    for(let i=0;i<count;i++) {
        const at=fieldArray+BigInt(i*fieldStride);
        if(i<selected) {
            for(let j=0;j<fieldStride;j++)memory.set(at+BigInt(j),memory.get(0x37000n+BigInt(i*fieldStride+j))??0);
        } else ptr(at+BigInt(mono?bytes:0),0x47000n);
    }
    ptr(owner+BigInt(fieldsOffset),fieldArray);
    const countOwner=mono&&kindOffset!==null?0x32000n:owner;
    number(countOwner+BigInt(countOffset),mono?4:2,count);
    const empty = at => {
        if(kind===0){ptr(at+BigInt(2*bytes),0);number(at+BigInt(3*bytes),4,0);}
        else for(const [name,offset] of outer)number(at+BigInt(offset),bytes,0);
    };
    empty(object);
    writeManagedArrayType(f, {mono,width,family,ptr,number}, 0x80000n, {kind:0x15});
    ptr(0x80000n+BigInt(2*bytes),0); ptr(0x80000n+BigInt(3*bytes),1);
    ptr(0x80000n+BigInt(4*bytes),object);
    let fieldReads=0;
    const read=f.process.read;
    f.process.read=request=>{
        const at=BigInt.asUintN(64,request.address);
        if(at>=fieldArray&&at<fieldArray+BigInt(count*fieldStride))fieldReads++;
        return read(request);
    };
    const host=await SplitScriptHost.instantiate(wasm),label=`${family}/${width}/${['list','map','set'][kind]}`;
    host.addProcess('game.exe',f.process);host.start();
    host.updateUntil(()=>host.variables.get('length')==='1',label,250);
    assert.equal(host.variables.get('error'),'');
    assert(fieldReads>=count,'seed must perform complete metadata discovery');
    // Four derived runtime classes share the same genuine corlib ancestor.
    // Runtime-class cache keys force independent layout discoveries.
    for(let i=0;i<4;i++){
        const klass=0x300000n+BigInt(i*0x1000),value=0x400000n+BigInt(i*0x1000),vtable=value+0x400n;
        for(let j=0;j<0x200;j++)memory.set(klass+BigInt(j),memory.get(root+BigInt(j))??0);
        ptr(value,mono?vtable:klass);ptr(vtable,klass);empty(value);
        ptr(0x80000n+BigInt(4*bytes+i*bytes),value);
    }
    ptr(0x80000n+BigInt(3*bytes),4);
    fieldReads=0;host.update();
    assert.match(host.variables.get('error'),/managed read work limit exceeded/,label);
    assert.equal(host.variables.get('length'),'1',`${label}: partial root was published`);
    assert(fieldReads>3*count&&fieldReads<4*count,`${label}: metadata work was not shared: ${fieldReads}`);
    maximumFieldReads=Math.max(maximumFieldReads,fieldReads);
    // Successful discoveries remain cached, while each retry receives a fresh
    // root context. Completed layouts no longer read their remote field table.
    host.updateUntil(()=>host.variables.get('length')==='4',`${label}: recover with a fresh root`,5);
    assert.equal(host.variables.get('error'),'');
    fieldReads=0;host.update();assert.equal(fieldReads,0,`${label}: cached layouts walked metadata again`);
    host.setProcessOpen('game.exe',false);host.update(2);
    host.addProcess('game.exe',f.process);fieldReads=0;
    host.updateUntil(()=>host.variables.get('error')?.includes('managed read work limit exceeded'),`${label}: attachment cache reset`,250);
    assert(fieldReads>3*count,`${label}: old attachment layouts were reused`);
    cases++;
}
console.log(JSON.stringify({managedMetadataBudgetCases:cases,maximumFieldReads}));
