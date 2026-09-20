import {writeGenericType, writeManagedArrayType} from './support/managed_type_fixture.mjs';
import assert from 'node:assert/strict';
import { SplitScriptHost } from './support/splitscript_host.mjs';
import { createKeyedCollectionFixture, writeKeyedBackingArray } from './support/keyed_collection_fixture.mjs';
const [wasm,backend]=process.argv.slice(2);
const modes=['backing nonvector','backing wrong kind','backing wrong class','backing empty wrong class','backing key kind','backing value kind','backing null class','backing unreadable class','torn array class','postscan array class','generic reference','generic inline','generic reference as value','generic value as reference','generic wrong width','generic null descriptor','generic unreadable descriptor','generic null class','generic unreadable flags','generic descriptor overflow','generic class overflow','holes','renamed','reversed','inline','empty allocated','empty null','all deleted','large capacity',
    'negative first count','negative second count','count ordering','too many live','too few live',
    'null entries','short entries','array bounds','capacity overflow','array address overflow',
    'unreadable hash','unreadable next','unreadable count','unreadable backing','scan budget','element budget','byte budget',
    'header budget','scan boundary','scan hard limit','byte boundary','zero value width','value overrun','key overrun','short reference',
    'short keys','short values','null keys','null values','torn count','torn backing','torn class',
    'postscan count','postscan backing','postscan class','full hash bits','retry','empty at address limit',
    'value kind mismatch','key kind mismatch','scalar wrong width',
    'inline wrong width','inline null class','inline unreadable class','inline zero size',
    'inline excessive size','inline unreadable type data','inline class overflow'];
let cases=0,maximumReads=0;
const boundaryTimings=[];
for(const family of backend==='mono'?['V1Cattrs','V2','V3']:['il2cpp'])for(const width of [32,64])for(const dictionary of [true,false])for(const parallel of [false,true])for(const mode of modes){
    if (!parallel && ['backing key kind','backing value kind'].includes(mode)) continue;
    if (!dictionary && mode === 'backing key kind') continue;
    if((!parallel||!dictionary)&&['short keys','null keys'].includes(mode))continue;
    if(!parallel&&['short values','null values'].includes(mode))continue;
    if(parallel&&['unreadable next','value overrun','short reference'].includes(mode))continue;
    if((parallel||!dictionary)&&mode==='key overrun')continue;
    if(!dictionary&&mode==='key kind mismatch')continue;
    const genericValue=['generic inline','generic value as reference','generic wrong width'].includes(mode);
    const f=createKeyedCollectionFixture({family,width,dictionary,parallel,reversed:mode==='reversed',renamed:mode==='renamed',inline:mode.startsWith('inline')||genericValue});
    const {memory,number,ptr,object,vtable,root,outer,stride,hash,next,key,value,bytes}=f;
    const arrays=[0x80000n,0x120000n,0x180000n], limit=(1n<<BigInt(width))-1n;
    const originalTypeData = Array.from({length:bytes},(_,i)=>BigInt(memory.get(f.valueType+BigInt(i))??0)<<BigInt(i*8)).reduce((a,b)=>a|b,0n);
    const valueClassSize=0x3a000n+BigInt(family==='il2cpp'?(width===64?0xf8:0x80):(width===64?0x1c:0x10));
    if(mode==='inline wrong width')number(valueClassSize,4,2*bytes+8);
    if(mode==='inline null class')ptr(f.valueType,0);
    if(mode==='inline unreadable class')memory.delete(valueClassSize);
    if(mode==='inline zero size')number(valueClassSize,4,2*bytes);
    if(mode==='inline excessive size')number(valueClassSize,4,2*bytes+1025);
    if(mode==='inline unreadable type data')memory.delete(f.valueType);
    if(mode==='inline class overflow')ptr(f.valueType,limit-1n);
    let generic;
    if(mode.startsWith('generic ')) {
        generic=writeGenericType({mono:family!=='il2cpp',width,ptr,number,cachedClass:f.valueClass},f.valueType,genericValue,mode==='generic wrong width'?8:16);
        // An unrelated generic definition size cannot determine instantiated storage.
        ptr(generic.descriptor,0x3c000n);number(0x3c000n+BigInt(width===64?0x1c:0x10),4,2*bytes+999);
        if(mode==='generic null descriptor')ptr(f.valueType,0);
        if(mode==='generic unreadable descriptor')memory.delete(generic.cached);
        if(mode==='generic null class')ptr(generic.cached,0);
        if(mode==='generic unreadable flags')memory.delete(generic.flags);
        if(mode==='generic descriptor overflow')ptr(f.valueType,limit-1n);
        if(mode==='generic class overflow')ptr(generic.cached,limit-1n);
    }
    let touched=4,live=2,capacity=6;
    if(mode==='empty allocated'||mode==='empty null'||mode==='empty at address limit'||mode==='backing empty wrong class'){touched=0;live=0;capacity=0;}
    if(mode==='all deleted')live=0;
    if(mode==='large capacity')capacity=65536;
    if(mode==='scan boundary'||mode==='scan hard limit'){touched=mode==='scan boundary'?4096:4097;live=1;capacity=touched;}
    if(mode==='empty at address limit')arrays.forEach((_,i)=>arrays[i]=limit-BigInt((i+1)*4*bytes)+1n);
    let keyBytes=f.keyBytes,valueBytes=f.valueBytes,scanBudget=4096,elementBudget=16384,byteBudget=1048576;
    if(mode==='scan hard limit'){scanBudget=0xffffffff;byteBudget=0xffffffffffffffffn;}
    if(mode==='zero value width')valueBytes=0;
    if(mode==='value overrun')valueBytes=stride+1;
    if(mode==='key overrun')keyBytes=stride;
    if(mode==='short reference')valueBytes=bytes-1;
    if(mode==='scan budget')scanBudget=touched-1;
    if(mode==='element budget')elementBudget=live-1;
    const cost=768+touched*(parallel?4:8)+live*64;
    if(mode==='byte budget')byteBudget=cost-1;
    if(mode==='byte boundary')byteBudget=cost;
    if(mode==='header budget')byteBudget=767;
    number(0x60010n,4,keyBytes);number(0x60014n,4,valueBytes);number(0x60018n,4,scanBudget);number(0x6001cn,4,elementBudget);number(0x60020n,8,byteBudget);number(0x60040n,1,0);
    number(0x60028n,4,mode==='key kind mismatch'?1<<0x1d:0xffffffff);
    number(0x6002cn,4,mode==='value kind mismatch'?1<<0x1d:0xffffffff);
    if(mode==='generic reference as value')number(0x6002cn,4,1<<0x11);
    if(mode==='generic value as reference'){number(0x6002cn,4,1<<0x12);number(0x60014n,4,bytes);number(generic.size,4,3*bytes);}
    if(mode==='scalar wrong width')number(f.valueType+BigInt(bytes+2),1,0x06);
    const countIndex=parallel?outer.length-2:2;
    const first=object+BigInt(outer[countIndex][1]),second=object+BigInt(outer[countIndex+1][1]);
    const setCounts=(t,l)=>{number(first,4,parallel||dictionary?t:l);number(second,4,parallel?l:dictionary?t-l:t);};
    setCounts(touched,live);
    const backingFields=outer.slice(1,parallel?outer.length-2:2).map(x=>object+BigInt(x[1]));
    const vector=(at,count)=>{ptr(at+BigInt(2*bytes),0);ptr(at+BigInt(3*bytes),count);};
    for(const [i,field]of backingFields.entries()){ptr(field,mode==='empty null'?0:arrays[i]);vector(arrays[i],capacity);writeKeyedBackingArray(f,arrays[i],i);}
    const originalBacking = writeKeyedBackingArray(f, arrays[0], 0);
    if (mode === 'backing nonvector') number(originalBacking.byval + BigInt(bytes + 2), 1, 0x12);
    if (mode === 'backing wrong kind') writeManagedArrayType(f, {mono:family!=='il2cpp',width,family,ptr,number}, arrays[0], 0x08);
    if (mode === 'backing null class') ptr(arrays[0], 0);
    if (mode === 'backing unreadable class') ptr(arrays[0], 0x27000000n);
    if (mode === 'backing key kind' || mode === 'backing value kind') {
        const index = mode === 'backing key kind' ? 1 : backingFields.length - 1;
        writeManagedArrayType(f, {mono:family!=='il2cpp',width,family,ptr,number}, arrays[index], 0x08);
    }
    if (mode === 'backing wrong class' || mode === 'backing empty wrong class') {
        for (let i = 0n; i < 0x200n; i++) memory.set(0x3e000n + i, memory.get(0x33000n + i) ?? 0);
        if (family !== 'il2cpp') ptr(originalBacking.byval, 0x3e000n);
        else {
            ptr(originalBacking.byval, 0x51000n);
            number(0x51000n + BigInt(bytes + 2), 1, 0x15);
            ptr(0x51000n, 0x51200n); ptr(0x51200n + BigInt(3 * bytes), 0x3e000n);
        }
    }
    const liveIndices=live===0?[]:live===1?[touched-1]:[0,3];
    for(let i=0;i<touched;i++){
        const at=arrays[0]+BigInt(4*bytes+i*stride),occupied=liveIndices.includes(i);
        let h=parallel?(occupied?0x80000042:0x42):(occupied?0x123:0xffffffff),n=-1;
        if(!parallel&&i===2&&!occupied){h=0x123;n=-2;}
        if(mode==='full hash bits'&&occupied)h=parallel?0xffffffff:0x80000000;
        if(mode==='too many live'&&i===1){h=parallel?0x80000042:0x123;n=-1;}
        if(mode==='too few live'&&i===3)h=parallel?0x42:0xffffffff;
        number(at+BigInt(hash),4,h);number(at+BigInt(next),4,n);
    }
    if(mode==='negative first count')number(first,4,-1);
    if(mode==='negative second count')number(second,4,-1);
    if(mode==='count ordering')number(parallel||!dictionary?first:second,4,parallel?1:touched+1);
    if(mode==='count ordering'&&parallel)number(second,4,2);
    if(mode==='null entries')ptr(backingFields[0],0);
    if(mode==='short entries')vector(arrays[0],touched-1);
    if(mode==='array bounds')ptr(arrays[0]+BigInt(2*bytes),1);
    if(mode==='capacity overflow')ptr(arrays[0]+BigInt(3*bytes),limit);
    if(mode==='array address overflow')ptr(backingFields[0],limit-1n);
    const firstHash=arrays[0]+BigInt(4*bytes+hash);
    if(mode==='unreadable hash'||mode==='retry')memory.delete(firstHash);
    if(mode==='unreadable next')memory.delete(arrays[0]+BigInt(4*bytes+next));
    if(mode==='unreadable count')memory.delete(first);
    if(mode==='unreadable backing')memory.delete(backingFields[0]);
    if(mode==='short keys')vector(arrays[1],touched-1);
    if(mode==='short values')vector(arrays[backingFields.length-1],touched-1);
    if(mode==='null keys')ptr(backingFields[1],0);
    if(mode==='null values')ptr(backingFields.at(-1),0);
    let reads=0,torn=false;
    const originalRead=f.process.read;
    f.process.read=request=>{
        const address=BigInt.asUintN(64,request.address);
        assert(address+BigInt(Math.max(request.length,1)-1)<=limit,`${mode}: overflowing host read`);
        if(address>=arrays[0]&&address<0x200000n){
            reads++;
            const marker=address>=arrays[0]+BigInt(4*bytes)&&address<arrays[0]+BigInt(4*bytes+touched*stride);
            if(marker){const offset=Number(address-arrays[0]-BigInt(4*bytes))%stride;assert(offset===hash||(!parallel&&offset===next),`${mode}: read a child payload`);}
            else assert(backingFields.some((_,i)=>address===arrays[i]||address===arrays[i]+BigInt(2*bytes)||address===arrays[i]+BigInt(3*bytes)),`${mode}: read spare capacity`);
        }
        const result=originalRead({...request,address});
        if(!torn&&((mode.startsWith('torn ')&&address===firstHash)||(mode.startsWith('postscan ')&&address===0x60040n))){
            torn=true;
            if(mode.endsWith('count'))number(first,4,99);
            if(mode.endsWith('backing'))ptr(backingFields[0],0);
            if (mode.endsWith('array class')) ptr(arrays[0], 0);
            else if(mode.endsWith('class'))ptr(family==='il2cpp'?object:vtable,root+0x100n);
        }
        return result;
    };
    const host=await SplitScriptHost.instantiate(wasm);host.addProcess('game.exe',f.process);
    const update=host.update.bind(host);
    host.update=(...args)=>{const begin=performance.now();update(...args);if(mode==='scan boundary'&&host.variables.has('result'))boundaryTimings.push(performance.now()-begin);};
    host.start();
    const label=`${family}/${width}/${dictionary?'map':'set'}/${parallel?'parallel':'entries'}/${mode}`;
    host.updateUntil(()=>host.variables.has('result'),label);
    const success=['generic reference','generic inline','holes','renamed','reversed','inline','empty allocated','empty at address limit','all deleted','large capacity','scan boundary','byte boundary','full hash bits'].includes(mode)||(mode==='empty null'&&!parallel);
    assert.equal(host.variables.get('result')==='ok',success,`${label}: ${host.variables.get('result')}`);
    if(success){
        const values=name=>(host.variables.get(name).match(/0x[0-9a-f]+|[0-9]+/gi)??[]).map(BigInt);
        const keys=liveIndices.map(i=>dictionary?(parallel?arrays[1]+BigInt(4*bytes+i*keyBytes):arrays[0]+BigInt(4*bytes+i*stride+key)):0n);
        const vals=liveIndices.map(i=>parallel?arrays[backingFields.length-1]+BigInt(4*bytes+i*valueBytes):arrays[0]+BigInt(4*bytes+i*stride+value));
        assert.deepEqual(values('keys'),keys,label);assert.deepEqual(values('values'),vals,label);
        assert.equal(host.variables.get('scanned'),String(touched),label);assert.equal(host.variables.get('bytes'),String(cost),label);
    }else assert(host.variables.get('result').length>0,label);
    if (mode.startsWith('backing ')) {
        assert.equal(reads <= backingFields.length + 1, true, `${label}: invalid array scanned payload`);
        if (mode.includes('wrong class')) assert.match(host.variables.get('result'), /element class differs/, label);
        if (mode.endsWith('kind')) assert.match(host.variables.get('result'), /element type differs/, label);
        for (const [i, array] of arrays.slice(0, backingFields.length).entries()) writeKeyedBackingArray(f, array, i);
        host.updateUntil(() => host.variables.get('result') === 'ok', `${label}: repaired backing type`);
    }
    if (mode.endsWith('array class')) assert.notEqual(host.variables.get('result'), 'ok', label);
    if(mode==='retry'){number(firstHash,4,parallel?0x80000042:0x123);host.updateUntil(()=>host.variables.get('result')==='ok',label);}
    if(['scan budget','element budget','byte budget','header budget','scan hard limit','negative first count','negative second count','count ordering'].includes(mode))assert.equal(reads,0,`${label}: budget/count failure read backing storage`);
    if(['value kind mismatch','key kind mismatch','scalar wrong width'].includes(mode)) {
        assert.match(host.variables.get('result'), /incompatible with/, label);
        assert.equal(reads,0,`${label}: invalid storage read payload`);
    }
    if(mode.startsWith('generic ')&&!success) {
        assert.equal(reads,0,`${label}: invalid generic storage read payload`);
        if(mode.endsWith('as value')||mode.endsWith('as reference'))assert.match(host.variables.get('result'),/type is incompatible/,label);
        if(mode==='generic wrong width')assert.match(host.variables.get('result'),/width is incompatible/,label);
        writeGenericType({mono:family!=='il2cpp',width,ptr,number,cachedClass:f.valueClass},f.valueType,genericValue);
        number(0x6002cn,4,0xffffffff);number(0x60014n,4,f.valueBytes);
        host.updateUntil(()=>host.variables.get('result')==='ok',`${label}: generic repair`);
    }
    if(mode.startsWith('inline ')) {
        assert.equal(reads,0,`${label}: invalid value type read payload`);
        if(mode==='inline wrong width')assert.match(host.variables.get('result'),/width is incompatible/,label);
        ptr(f.valueType,originalTypeData);number(valueClassSize,4,2*bytes+16);
        host.updateUntil(()=>host.variables.get('result')==='ok',`${label}: recover metadata`);
    }
    maximumReads=Math.max(maximumReads,reads);cases++;
}
boundaryTimings.sort((a,b)=>a-b);
console.log(JSON.stringify({backend,keyedSlotCases:cases,maximumPayloadReads:maximumReads,scanBoundaryMs:{median:boundaryTimings[Math.floor(boundaryTimings.length/2)],maximum:boundaryTimings.at(-1)}}));
