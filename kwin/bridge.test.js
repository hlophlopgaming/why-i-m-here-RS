const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
function signal() { const callbacks=[]; return {connect(fn){callbacks.push(fn);}, emit(...args){callbacks.forEach(fn=>fn(...args));}}; }
function window(id, app, pid=2) {
	let minimized=false;
	const changed=signal();
	return {internalId:id, desktopFileName:app, pid, normalWindow:true, minimizable:true,
		caption:'fixture', minimizedChanged:changed,windowClassChanged:signal(),desktopFileNameChanged:signal(),
		get minimized(){return minimized;},set minimized(value){if(value!==minimized){minimized=value;changed.emit();}},
		closed:0,closeWindow(){this.closed++;}};
}
function run() {
	const timers=[];
	function QTimer(){this.timeout=signal();this.active=false;this.start=()=>{this.active=true;};this.stop=()=>{this.active=false;};timers.push(this);}
	const a=window('one','fixture'), b=window('two','fixture');
	const other=window('other','unselected');
	const own=window('ui','org.local.WhyHere',3);own.caption='Why am I here? — Fixture';
	const workspace={windowAdded:signal(),windowRemoved:signal(),windowActivated:signal(),windowList:()=>[a,b,other,own]};
	let reply={blocked:['desktop:fixture.desktop'],close:[],focus:{"desktop:fixture.desktop":3},running:true,watch:true};
	const snapshots=[];
	const source=fs.readFileSync(__dirname+'/bridge.js','utf8').replace('__WHY_HERE_CONFIG__',JSON.stringify({pid:1,service:'test',blocked:reply.blocked,apps:[{id:reply.blocked[0],desktop:'fixture',file:'fixture.desktop'}]}));
	vm.runInNewContext(source,{workspace,QTimer,callDBus(service,path,iface,method,version,pid,json,released,callback){
		assert.equal(version,'1');assert.equal(pid,'1');
		snapshots.push(JSON.parse(json));if(callback) callback(JSON.stringify(reply));
	}});
	assert.equal(a.minimized,true);assert.equal(b.minimized,true);assert.equal(other.minimized,false);
	assert.equal(snapshots[0].length,2);
	a.minimized=false;assert.equal(a.minimized,true);assert.equal(workspace.activeWindow,own);
	workspace.activeWindow=other;workspace.windowActivated.emit(other);assert.equal(workspace.activeWindow,other);
	reply={...reply,blocked:[]};timers[0].timeout.emit();assert.equal(a.minimized,false);assert.equal(b.minimized,false);
	reply={...reply,close:['one','two'],watch:false};timers[0].timeout.emit();assert.equal(a.closed,1);assert.equal(b.closed,1);assert.equal(other.closed,0);
	reply={...reply,blocked:['desktop:fixture.desktop'],close:[],watch:true};timers[0].timeout.emit();
	assert.equal(a.minimized,true);timers[1].timeout.emit();assert.equal(a.minimized,false);assert.equal(b.minimized,false);
	assert.equal(timers[0].active,false);
	const later=window("later","fixture");workspace.windowAdded.emit(later);assert.equal(later.minimized,false);
}
run();
console.log('KWin bridge checks passed');
