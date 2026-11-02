import { describe, it, expect } from 'vitest';

describe('native DOM interface ownership', () => {
  it('keeps native shadow-root identity, ownership and scoped queries', () => {
    const host=document.createElement('div');document.body.appendChild(host);
    const root=host.attachShadow({mode:'open'});
    expect(root.nodeType).toBe(11);
    expect(Object.getPrototypeOf(root)).toBe(ShadowRoot.prototype);
    expect(Object.getPrototypeOf(ShadowRoot.prototype)).toBe(DocumentFragment.prototype);
    expect(root.host).toBe(host);
    expect(root.mode).toBe('open');
    expect(root.ownerDocument).toBe(document);
    expect(host.shadowRoot).toBe(root);
    expect(root.parentNode).toBe(null);
    root.innerHTML='<span id="native-shadow">text</span>';
    const span=root.querySelector('#native-shadow');
    expect(span.textContent).toBe('text');
    expect(document.querySelector('#native-shadow')).toBe(null);
    expect(span.getRootNode()).toBe(root);
    expect(span.getRootNode({composed:true})).toBe(document);
    const errorName=operation=>{try{operation();return null;}catch(error){return error.name;}};
    expect(errorName(()=>root.cloneNode(true))).toBe('NotSupportedError');
    expect(errorName(()=>host.attachShadow({mode:'open'}))).toBe('NotSupportedError');
    expect(document.body.appendChild(root)).toBe(root);
    expect(root.childNodes.length).toBe(0);
    expect(span.parentNode).toBe(document.body);
    expect(span.getRootNode()).toBe(document);
    expect(root.parentNode).toBe(null);
    expect(root.host).toBe(host);
    expect(root.ownerDocument).toBe(document);
    expect(host.shadowRoot).toBe(root);
    span.remove();
    const closedHost=document.createElement('div'),closed=closedHost.attachShadow({mode:'closed'});
    expect(closed.mode).toBe('closed');expect(closed.host).toBe(closedHost);expect(closedHost.shadowRoot).toBe(null);
    host.remove();
  });

  it('owns node constants and event methods on interfaces with real listener state', () => {
    const element=document.createElement('div');
    expect(Object.hasOwn(element,'ELEMENT_NODE')).toBe(false);
    expect(element.ELEMENT_NODE).toBe(Node.ELEMENT_NODE);
    expect(Node.ELEMENT_NODE).toBe(1);
    expect(Node.DOCUMENT_FRAGMENT_NODE).toBe(11);
    const constant=Object.getOwnPropertyDescriptor(Node.prototype,'TEXT_NODE');
    expect(constant.value).toBe(3);expect(constant.writable).toBe(false);expect(constant.configurable).toBe(false);expect(constant.enumerable).toBe(true);
    expect(element.__cascadeProp).toBe(undefined);
    expect(element.activeElement).toBe(undefined);
    expect(element.checkValidity).toBe(undefined);
    expect(Object.hasOwn(element,'addEventListener')).toBe(false);
    expect(element.addEventListener).toBe(EventTarget.prototype.addEventListener);
    let dom=0,custom=0,windowEvents=0;
    element.addEventListener('event',()=>dom++);
    element.dispatchEvent(new Event('event'));
    class Bus extends EventTarget {}
    const bus=new Bus();bus.addEventListener('event',()=>custom++);bus.dispatchEvent(new Event('event'));
    const listener=()=>windowEvents++;
    EventTarget.prototype.addEventListener.call(window,'event',listener);
    EventTarget.prototype.dispatchEvent.call(window,new Event('event'));
    EventTarget.prototype.removeEventListener.call(window,'event',listener);
    expect([dom,custom,windowEvents]).toEqual([1,1,1]);
    expect(()=>EventTarget.prototype.addEventListener.call({},'event',listener)).toThrow();
    expect(()=>EventTarget()).toThrow();
    const input=document.createElement('input');
    expect(input.checkValidity()).toBe(true);
    expect(Object.hasOwn(input,'checkValidity')).toBe(false);
  });

  it('uses Web IDL constructor inheritance and property flags', () => {
    // Genuine Chrome permits Document, Text and DocumentFragment construction;
    // the HTML element interfaces still require their browser creation paths.
    for(const [child,parent,constructible] of [[Node,EventTarget,false],[Element,Node,false],[HTMLElement,Element,false],[HTMLDivElement,HTMLElement,false],[Document,Node,true],[HTMLDocument,Document,false],[DocumentFragment,Node,true],[Text,CharacterData,true],[HTMLAudioElement,HTMLMediaElement,false]] as const){
      expect(Object.getPrototypeOf(child)).toBe(parent);
      expect(Object.getPrototypeOf(child.prototype)).toBe(parent.prototype);
      expect(Object.getOwnPropertyDescriptor(child,'prototype').writable).toBe(false);
      expect(Object.getOwnPropertyDescriptor(globalThis,child.name).enumerable).toBe(false);
      expect(()=>child()).toThrow(TypeError);
      if(!constructible){expect(()=>new child()).toThrow(TypeError);continue;}
      const node=new child();
      expect(Object.getPrototypeOf(node)).toBe(child.prototype);
      expect(node.parentNode).toBe(null);
      expect(node.childNodes.length).toBe(0);
      if(child===Document){
        expect(node.nodeType).toBe(9);expect(node.nodeName).toBe('#document');
        expect(node.ownerDocument).toBe(null);expect(node.defaultView).toBe(null);
        expect(node.documentElement).toBe(null);expect(node.contentType).toBe('application/xml');
      }else{
        expect(node.ownerDocument).toBe(document);
        expect(node.nodeType).toBe(child===Text?3:11);
        expect(node.nodeName).toBe(child===Text?'#text':'#document-fragment');
        if(child===Text)expect(node.data).toBe('');
      }
    }
  });

  it('uses real fragments that flatten children when inserted', () => {
    const fragment=document.createDocumentFragment();
    expect(fragment.nodeType).toBe(11);
    expect(fragment.nodeName).toBe('#document-fragment');
    expect(Object.getPrototypeOf(fragment)).toBe(DocumentFragment.prototype);
    expect(fragment instanceof Node).toBe(true);
    expect(fragment instanceof Element).toBe(false);
    const span=document.createElement('span');span.textContent='fragment';
    const text=document.createTextNode(' text');
    fragment.append(span,text);
    expect(fragment.children.length).toBe(1);
    expect(fragment.firstElementChild).toBe(span);
    expect(fragment.lastElementChild).toBe(span);
    expect(fragment.childElementCount).toBe(1);
    expect(fragment.querySelector('span')).toBe(span);
    expect(fragment.textContent).toBe('fragment text');
    const clone=fragment.cloneNode(true);
    expect(clone.nodeType).toBe(11);
    expect(clone.textContent).toBe('fragment text');
    const parent=document.createElement('div');
    expect(parent.appendChild(fragment)).toBe(fragment);
    expect(fragment.childNodes.length).toBe(0);
    expect(parent.firstChild).toBe(span);
    expect(parent.childElementCount).toBe(1);
    expect(parent.textContent).toBe('fragment text');
    const second=document.createDocumentFragment();second.append(document.createTextNode('before'));
    parent.insertBefore(second,span);
    expect(second.childNodes.length).toBe(0);
    expect(parent.textContent).toBe('beforefragment text');
  });

  it('constructs native-backed class delegates with subclass state and new.target', () => {
    expect((globalThis as any)[Symbol.for('ts')]).toBe(undefined);
    class Child extends CustomEvent<number> { target2: unknown; constructor(type: string, init?: CustomEventInit<number>){super(type,init);this.target2=new.target;} }
    const original=Reflect.construct;
    try{
      Reflect.construct=()=>{throw new Error('modified');};
      const instance=new Child('ping',{detail:7});
      expect(instance.type).toBe('ping');
      expect(instance.detail).toBe(7);
      expect(instance.target2).toBe(Child);
      expect(instance instanceof Child).toBe(true);
      expect(instance instanceof CustomEvent).toBe(true);
      expect(instance instanceof Event).toBe(true);
      expect(()=>(CustomEvent as any)('ping')).toThrow(TypeError);
      expect(()=>new (CustomEvent as any)()).toThrow(TypeError);
    }finally{Reflect.construct=original;}
  });

  it('keeps distinct interface inheritance and branding', () => {
    expect(Object.getPrototypeOf(Node.prototype)).toBe(EventTarget.prototype);
    expect(Object.getPrototypeOf(Element.prototype)).toBe(Node.prototype);
    expect(Object.getPrototypeOf(HTMLElement.prototype)).toBe(Element.prototype);
    expect(Object.getPrototypeOf(HTMLDivElement.prototype)).toBe(HTMLElement.prototype);
    expect(Object.getPrototypeOf(Document.prototype)).toBe(Node.prototype);
    expect(Object.getPrototypeOf(HTMLDocument.prototype)).toBe(Document.prototype);
    expect(Object.getPrototypeOf(Text.prototype)).toBe(CharacterData.prototype);
    expect(Object.getPrototypeOf(CharacterData.prototype)).toBe(Node.prototype);
    const element=document.createElement('div');
    expect(Object.getPrototypeOf(element)).toBe(HTMLDivElement.prototype);
    expect(Object.prototype.toString.call(element)).toBe('[object HTMLDivElement]');
    expect(Object.prototype.toString.call(document)).toBe('[object HTMLDocument]');
    expect(Object.prototype.toString.call(document.createTextNode('text'))).toBe('[object Text]');
  });

  it('exposes receiver-safe native accessors on their owning interfaces', () => {
    const element=document.createElement('div');
    expect(Object.hasOwn(element,'id')).toBe(false);
    expect(Object.hasOwn(element,'nodeType')).toBe(false);
    const id=Object.getOwnPropertyDescriptor(Element.prototype,'id');
    const type=Object.getOwnPropertyDescriptor(Node.prototype,'nodeType');
    expect(id.enumerable).toBe(true);
    expect(id.configurable).toBe(true);
    expect(id.get.name).toBe('get id');
    expect(id.get.length).toBe(0);
    expect(id.set.name).toBe('set id');
    expect(id.set.length).toBe(1);
    expect(Object.hasOwn(id.get,'prototype')).toBe(false);
    expect(()=>id.get.call({})).toThrow();
    id.set.call(element,'native-id');
    expect(element.getAttribute('id')).toBe('native-id');
    expect(id.get.call(element)).toBe('native-id');
    expect(type.get.call(element)).toBe(1);
    element.innerHTML='<span>live</span>';
    expect(element.firstChild.parentNode).toBe(element);
    expect(element.textContent).toBe('live');
  });

  it('shares native prototype methods while preserving tree and expando identity', () => {
    const first=document.createElement('div'),second=document.createElement('div');
    expect(Object.hasOwn(first,'appendChild')).toBe(false);
    expect(Object.hasOwn(first,'getAttribute')).toBe(false);
    expect(first.appendChild).toBe(Node.prototype.appendChild);
    expect(first.getAttribute).toBe(Element.prototype.getAttribute);
    first.__marker=42;
    const text=document.createTextNode('live');
    first.appendChild(text);
    second.appendChild(first);
    expect(second.firstChild).toBe(first);
    expect(second.firstChild.__marker).toBe(42);
    expect(first.firstChild).toBe(text);
    expect(text.parentNode).toBe(first);
    expect(first.cloneNode(true).textContent).toBe('live');
  });
});
it('keeps document collections and native creation methods on Document while Node operations mutate the real tree',()=>{
  document.body.innerHTML='<section class="container"><i class="target" name="token"></i><b></b></section>';
  const section=document.body.firstElementChild, first=section.firstChild, second=section.lastChild;
  expect(document.getElementsByTagName('i')[0]).toBe(first);
  expect(document.getElementsByClassName('target')[0]).toBe(first);
  expect(document.getElementsByName('token')[0]).toBe(first);
  expect(Object.hasOwn(Document.prototype,'getElementsByTagName')).toBe(true);
  expect(Object.hasOwn(Element.prototype,'getElementsByTagName')).toBe(true);
  expect(Object.hasOwn(HTMLElement.prototype,'getElementsByTagName')).toBe(false);
  expect(Object.hasOwn(document,'createTextNode')).toBe(false);
  expect(Object.hasOwn(Node.prototype,'lastChild')).toBe(true);
  expect(Object.hasOwn(Node.prototype,'previousSibling')).toBe(true);
  expect(second.previousSibling).toBe(first);
  expect(section.compareDocumentPosition(first)).toBe(20);
  expect(first.compareDocumentPosition(section)).toBe(10);
  expect(first.compareDocumentPosition(second)).toBe(4);
  expect(second.compareDocumentPosition(first)).toBe(2);
  const detached=document.createElement('div');
  expect(first.compareDocumentPosition(detached)&33).toBe(33);
  expect((first.compareDocumentPosition(detached)&6) !== (detached.compareDocumentPosition(first)&6)).toBe(true);
  const fragment=document.createDocumentFragment();fragment.appendChild(document.createTextNode(''));fragment.appendChild(document.createTextNode('one'));fragment.appendChild(document.createTextNode('two'));
  fragment.normalize();expect(fragment.childNodes.length).toBe(1);expect(fragment.firstChild.data).toBe('onetwo');
  const replacement=document.createTextNode('replacement');
  expect(section.replaceChild(replacement,second)).toBe(second);
  expect(second.parentNode).toBe(null);expect(section.lastChild).toBe(replacement);
  expect(()=>Node.prototype.compareDocumentPosition.call({},first)).toThrow(TypeError);
  expect(()=>Document.prototype.getElementsByTagName.call(section,'i')).toThrow(TypeError);
  const canvas=document.createElement('canvas');expect('value' in canvas).toBe(false);expect(typeof canvas.getContext).toBe('function');
});
it('stores element property handlers privately and dispatches them once with live event state',()=>{
  const parent=document.createElement('div'),child=document.createElement('button');parent.appendChild(child);
  expect(child.onclick).toBe(null);expect(Object.hasOwn(child,'onclick')).toBe(false);
  const seen=[];
  child.addEventListener('click',event=>seen.push(['listener',event.currentTarget===child,event.eventPhase,event.isTrusted]));
  child.onclick=function(event){seen.push(['property',this===child,event.eventPhase,event.isTrusted]);return false;};
  parent.onclick=function(event){seen.push(['parent',this===parent,event.eventPhase,event.target===child]);};
  const event=new Event('click',{bubbles:true,cancelable:true});
  expect(child.dispatchEvent(event)).toBe(false);
  expect(seen).toEqual([['listener',true,2,false],['property',true,2,false],['parent',true,3,true]]);
  expect(event.currentTarget).toBe(null);expect(event.eventPhase).toBe(0);expect(event.defaultPrevented).toBe(true);
  child.onclick=42;expect(child.onclick).toBe(null);
  const getter=Object.getOwnPropertyDescriptor(HTMLElement.prototype,'onclick').get;
  expect(()=>getter.call({})).toThrow(TypeError);expect(()=>new getter()).toThrow(TypeError);
  const ordered=document.createElement('span'),order=[];ordered.onclick=()=>order.push('first');ordered.addEventListener('click',()=>order.push('second'));ordered.onclick=()=>order.push('replacement');ordered.dispatchEvent(new Event('click'));expect(order).toEqual(['replacement','second']);
  ordered.onclick=null;ordered.onclick=()=>order.push('last');ordered.dispatchEvent(new Event('click'));expect(order.slice(2)).toEqual(['second','last']);
  const stopped=document.createElement('span');let called=false;stopped.addEventListener('click',event=>event.stopImmediatePropagation());stopped.onclick=()=>{called=true;};stopped.dispatchEvent(new Event('click'));expect(called).toBe(false);
});
it('reads parsed and created SVG and MathML namespace identity from the native tree',()=>{
  document.body.innerHTML='<svg><circle id="round"/></svg><math><mi>x</mi></math>';
  const svg=document.body.firstElementChild,circle=svg.firstElementChild,math=svg.nextElementSibling;
  expect(svg.namespaceURI).toBe('http://www.w3.org/2000/svg');expect(circle.namespaceURI).toBe(svg.namespaceURI);
  expect(Object.getPrototypeOf(svg)).toBe(SVGSVGElement.prototype);expect(Object.getPrototypeOf(circle)).toBe(SVGElement.prototype);
  expect(math.namespaceURI).toBe('http://www.w3.org/1998/Math/MathML');
  const created=document.createElementNS('http://www.w3.org/2000/svg','path');created.setAttribute('viewBox','0 0 2 2');
  expect(created.namespaceURI).toBe(svg.namespaceURI);expect(Object.getPrototypeOf(created)).toBe(SVGElement.prototype);
  expect(created.getAttribute('viewBox')).toBe('0 0 2 2');expect(created.cloneNode(true).namespaceURI).toBe(svg.namespaceURI);
  expect(document.body.namespaceURI).toBe('http://www.w3.org/1999/xhtml');
});
it('resolves native namespace prefixes through ancestors without treating plain xmlns attributes as namespaced declarations',()=>{
  const ns='http://www.w3.org/2000/svg',root=document.createElementNS(ns,'s:svg'),child=document.createElementNS(ns,'s:g');root.appendChild(child);
  expect(root.prefix).toBe('s');expect(root.localName).toBe('svg');expect(root.tagName).toBe('s:svg');expect(root.nodeName).toBe('s:svg');
  root.setAttribute('xmlns:other','urn:example');
  expect(child.lookupNamespaceURI('s')).toBe(ns);expect(child.lookupPrefix(ns)).toBe('s');
  expect(child.lookupNamespaceURI('other')).toBe(null);expect(child.lookupPrefix('urn:example')).toBe(null);
  child.setAttribute('xmlns:other','urn:replacement');expect(child.lookupPrefix('urn:example')).toBe(null);
  const text=document.createTextNode('x');child.appendChild(text);expect(text.lookupNamespaceURI('other')).toBe(null);expect(text.lookupNamespaceURI('s')).toBe(ns);
  expect(document.isDefaultNamespace('http://www.w3.org/1999/xhtml')).toBe(true);expect(root.isDefaultNamespace(ns)).toBe(false);
  expect(document.createDocumentFragment().lookupNamespaceURI(null)).toBe(null);
  expect(()=>Node.prototype.lookupNamespaceURI.call({},null)).toThrow(TypeError);
});
it('reflects real native attributes through shared ARIA and HTML properties',()=>{
  const element=document.createElement('div');expect(element.role).toBe(null);expect(element.ariaLabel).toBe(null);
  element.ariaLabel='named';expect(element.getAttribute('aria-label')).toBe('named');
  element.setAttribute('aria-colindextext','column three');expect(element.ariaColIndexText).toBe('column three');
  element.ariaLabel=null;expect(element.hasAttribute('aria-label')).toBe(false);element.role=undefined;expect(element.role).toBe(null);
  element.title=null;expect(element.title).toBe('null');element.lang='EN';element.accessKey='x';
  expect(element.getAttributeNames()).toEqual(['aria-colindextext','title','lang','accesskey']);expect(element.hasAttributes()).toBe(true);
  const get=Object.getOwnPropertyDescriptor(Element.prototype,'ariaLabel').get;expect(()=>get.call({})).toThrow(TypeError);expect(()=>element.title=Symbol()).toThrow(TypeError);
  element.innerHTML='<b>inside</b>';element.insertAdjacentText('afterbegin','<literal>');element.insertAdjacentText('beforeend','last');
  expect(element.firstChild.nodeType).toBe(3);expect(element.firstChild.data).toBe('<literal>');expect(element.lastChild.data).toBe('last');
  const fragment=document.createDocumentFragment();fragment.appendChild(element);element.insertAdjacentText('beforebegin','before');element.insertAdjacentText('afterend','after');
  expect(fragment.firstChild.data).toBe('before');expect(fragment.lastChild.data).toBe('after');expect(()=>element.insertAdjacentText('bad','x')).toThrow();
});
it('keeps link, stylesheet, script and numeric-control properties on their actual interfaces',()=>{
  const div=document.createElement('div'),script=document.createElement('script'),link=document.createElement('link'),input=document.createElement('input');
  for(const name of ['src','href','rel','as','media','type','sheet','valueAsNumber'])if(name in div)throw new Error('Unexpected inherited property: '+name);
  script.src='/asset.js';expect(script.getAttribute('src')).toBe('/asset.js');expect(Object.hasOwn(HTMLScriptElement.prototype,'src')).toBe(true);
  link.rel='stylesheet';link.href='/asset.css';link.as='style';link.media='screen';expect(link.getAttribute('href')).toBe('/asset.css');expect(link.getAttribute('rel')).toBe('stylesheet');
  expect(Object.hasOwn(HTMLLinkElement.prototype,'sheet')).toBe(true);expect(link.sheet).toBe(null);
  const style=document.createElement('style');style.textContent='sentinel {}';document.head.appendChild(style);expect(style.sheet.cssRules[0].selectorText).toBe('sentinel');style.remove();
  input.type='number';input.value='42';expect(input.valueAsNumber).toBe(42);input.valueAsNumber=7;expect(input.value).toBe('7');
  const src=Object.getOwnPropertyDescriptor(HTMLScriptElement.prototype,'src').get;expect(()=>src.call(div)).toThrow(TypeError);
});
