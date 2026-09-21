import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {SplitScriptHost} from './support/splitscript_host.mjs';
import {createMonoPeFixture} from './support/mono_pe_fixture.mjs';
import {createIl2cppPeFixture} from './support/il2cpp_pe_fixture.mjs';

const [wasm] = process.argv.slice(2);
const profiles = JSON.parse(await readFile(new URL('./fixtures/mono-pe-profiles.json', import.meta.url)));
const modes = ['valid', 'named namespace', 'late image', 'late name', 'late count', 'late class',
    'class name null', 'class name unreadable', 'image overflow', 'name overflow',
    'assembly overflow', 'class overflow', 'class name overflow', 'table overflow',
    'excessive count', 'large', 'large retry', 'large images', 'large images retry', 'cancel'];
let cases = 0, peakReads = 0;
for (const route of ['mono image', 'mono assembly', 'il2cpp image']) for (const width of [32, 64]) {
    const mono = route.startsWith('mono'), exact = route !== 'mono assembly', wide = width === 64;
    const bytes = width / 8, p = BigInt(bytes), limit = (1n << BigInt(width)) - 1n;
    const make = () => mono
        ? createMonoPeFixture(exact ? profiles.builds.find(p => p.width === width && p.version === 'V2') : {width, version: 'V2', exact: false, playerVersion: [2020, 1]})
        : createIl2cppPeFixture({width, version: [2022, 3, 0, 37029]});
    for (const mode of modes) {
        if ((mode === 'large retry' || mode.startsWith('large images')) && mono) continue;
        const f = make(), {memory} = f;
        const write = (at, size, value) => {
            const data = new Uint8Array(size), v = new DataView(data.buffer);
            if (size === 8) v.setBigUint64(0, BigInt(value), true); else v.setUint32(0, Number(value), true);
            data.forEach((b,i) => {
                const address=at+BigInt(i);
                if(address>=f.base&&address<f.base+BigInt(f.image.length))f.image[Number(address-f.base)]=b;
                else memory.set(address,b);
            });
        };
        const ptr = (at,value) => write(at,bytes,value);
        const text = (at,value) => {
            const data = new Uint8Array(256); data.set(new TextEncoder().encode(value));
            data.forEach((b,i) => memory.set(at + BigInt(i), b));
        };
        const assembly = 0x11000n, image = 0x12000n, table = 0x13000n, klass = 0x14000n;
        const imageSlot = assembly + BigInt(mono ? (wide ? 0x60 : 0x44) : 0);
        const nameSlot = route === 'mono assembly' ? assembly + BigInt(wide ? 0x10 : 8)
            : image + BigInt(mono ? (wide ? 0x28 : 0x18) : bytes);
        const className = BigInt(mono ? (wide ? 0x48 : 0x2c) : (wide ? 0x10 : 8));
        const next = BigInt(wide ? 0x108 : 0xa8);
        const countSlot = image + BigInt(mono ? (wide ? 0x4d8 : 0x360) : (wide ? 0x18 : 0xc));
        const tableSlot = mono ? image + BigInt(wide ? 0x4e0 : 0x368) : f.typeInfo;
        const classSlot = table + (mono ? 0n : 8n * p);
        const pristine = new Map(memory);
        // Unqualified source names match a class in any runtime namespace.
        if (mode === 'named namespace') text(0x23000n, 'Game.Runtime');
        if (mode === 'late image') ptr(imageSlot,0);
        if (mode === 'late name') ptr(nameSlot,0);
        if (mode === 'late count') write(countSlot,4,0);
        if (mode === 'late class') ptr(classSlot,0);
        if (mode === 'class name null') ptr(klass + className,0);
        if (mode === 'class name unreadable') memory.delete(0x22000n);
        if (mode === 'image overflow') ptr(imageSlot,limit-1n);
        if (mode === 'name overflow') ptr(nameSlot,limit-127n);
        if (mode === 'assembly overflow') ptr(0x10000n,limit-1n);
        if (mode === 'class overflow') ptr(classSlot,limit-1n);
        if (mode === 'class name overflow') ptr(klass + className,limit-127n);
        if (mode === 'table overflow') ptr(tableSlot,limit-1n);
        if (mode === 'excessive count') write(countSlot,4,1048577);
        if (mode === 'large' || mode === 'large retry' || mode === 'cancel') {
            text(0x70000n,'Unrelated');
            if (mono) ptr(table,0x40000n); else write(countSlot,4,193);
            for (let i=0; i<192; i++) {
                const at = 0x40000n + BigInt(i*0x200);
                ptr(at+className,0x70000n);
                if (mono) ptr(at+next,i===191 ? klass : at+0x200n);
                else ptr(table+BigInt(7+i)*p,at);
            }
            if (!mono) ptr(table+199n*p,klass);
        }
        const lateName = 0x40000n + 130n * 0x200n + className;
        if (mode === 'large retry') ptr(lateName,0);
        if (mode.startsWith('large images')) {
            ptr(f.assemblies+p,0x10000n+193n*p);
            ptr(0x80000n,0x81000n);
            ptr(0x81000n+p,0x82000n);text(0x82000n,'Unrelated');
            for (let i=0;i<192;i++) ptr(0x10000n+BigInt(i)*p,0x80000n);
            ptr(0x10000n+192n*p,assembly);
            if (mode==='large images retry') {
                ptr(0x10000n+130n*p,0x84000n);ptr(0x84000n,0);
            }
        }
        let reads=0, classReads=0, imageReads=0;
        const read=f.process.read;
        f.process.read=request => {
            const address=BigInt.asUintN(64,request.address);
            assert(address>=0x1000n && address+BigInt(request.length)-1n<=limit,
                `${route}/${width}/${mode}: wrapped or out-of-range host read ${address.toString(16)}/${request.length}`);
            reads++;
            if (address===0x80000n) imageReads++;
            if (address>=0x40000n && address<0x58000n) classReads++;
            return read({...request,address});
        };
        const host=await SplitScriptHost.instantiate(wasm);
        host.addProcess('game.exe',f.process);host.start();
        const label=`${route}/${width}/${mode}`;
        let ticks=0, maximum=0;
        const update=()=>{const before=reads;host.update();maximum=Math.max(maximum,reads-before);ticks++;};
        if (mode==='valid'||mode==='named namespace'||mode==='large'||mode==='large images') {
            while (!host.messages.includes('42')&&ticks<80) update();
            assert(host.messages.includes('42'),label);
            if (mode==='large'||mode==='large images') assert(ticks>=4,`${label}: monopolized one update`);
        } else {
            if (mode==='cancel') {
                while (classReads<64&&ticks<80) update();
                assert(classReads>=64,`${label}: never entered class traversal`);
            } else while(ticks<30) update();
            assert(!host.messages.includes('42'),`${label}: accepted incomplete metadata`);
            const retry = mode.startsWith('late ') || ['class name null','class name unreadable','name overflow','class name overflow'].includes(mode)
                || mode==='image overflow'&&route!=='mono assembly';
            if (mode === 'large retry') {
                assert(classReads >= 128,`${label}: never reached a later batch`);
                ptr(lateName,0x70000n);
                host.updateUntil(()=>host.messages.includes('42'),`${label}: resume repaired batch`,100);
            } else if (mode === 'large images retry') {
                assert(imageReads>=128,`${label}: never reached a later batch`);
                ptr(0x84000n,0x81000n);
                host.updateUntil(()=>host.messages.includes('42'),`${label}: resume repaired image batch`,100);
            } else if (retry) {
                memory.clear();for(const [at,b]of pristine)memory.set(at,b);
                host.updateUntil(()=>host.messages.includes('42'),`${label}: repair without reattachment`,100);
            } else {
                host.setProcessOpen('game.exe',false);host.update(3);
                host.addProcess('game.exe',make().process);
                host.updateUntil(()=>host.messages.includes('42'),`${label}: fresh attachment`,100);
            }
        }
        assert(maximum<600,`${label}: ${maximum} reads in one update`);
        peakReads=Math.max(peakReads,maximum);cases++;
    }
}
console.log(JSON.stringify({metadataBoundsCases:cases,peakReads}));
