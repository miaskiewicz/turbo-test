'use strict';
const assert=require('node:assert/strict');
const buffer=require('node:buffer');

test('Buffer module constants are immutable engine limits and are not constructor statics',()=>{
 assert.equal(require('buffer'),buffer);
 assert.equal(Buffer.constants,undefined);
 const descriptor=Object.getOwnPropertyDescriptor(buffer,'constants');
 assert.equal(typeof descriptor.value,'object');
 assert.equal(descriptor.enumerable,true);
 assert.equal(descriptor.writable,false);
 assert.equal(descriptor.configurable,false);
 assert.equal(Object.isExtensible(buffer.constants),true);
 assert.equal(Object.getPrototypeOf(buffer.constants),Object.prototype);
 for(const name of ['MAX_LENGTH','MAX_STRING_LENGTH']){
  const limit=Object.getOwnPropertyDescriptor(buffer.constants,name);
  assert.equal(typeof limit.value,'number');
  assert.ok(Number.isSafeInteger(limit.value)&&limit.value>0);
  assert.equal(limit.enumerable,true);
  assert.equal(limit.writable,false);
  assert.equal(limit.configurable,false);
  assert.throws(()=>{buffer.constants[name]=1},TypeError);
 }
 assert.throws(()=>{buffer.constants={}},TypeError);
});

test('Buffer legacy limit aliases retain Node module property descriptors',()=>{
 for(const [name,key] of [['kMaxLength','MAX_LENGTH'],['kStringMaxLength','MAX_STRING_LENGTH']]){
  const property=Object.getOwnPropertyDescriptor(buffer,name);
  assert.equal(property.value,buffer.constants[key]);
  assert.equal(property.enumerable,true);
  assert.equal(property.writable,true);
  assert.equal(property.configurable,true);
 }
 const copied={};
 for(const name in buffer)if(Object.prototype.hasOwnProperty.call(buffer,name))copied[name]=buffer[name];
 assert.equal(copied.constants.MAX_STRING_LENGTH,buffer.constants.MAX_STRING_LENGTH);
});

test('reported limits reject oversized lengths without allocating the requested memory',()=>{
 assert.throws(()=>Buffer.alloc(buffer.constants.MAX_LENGTH+1),RangeError);
 assert.throws(()=>'x'.repeat(buffer.constants.MAX_STRING_LENGTH+1),RangeError);
 assert.equal(Buffer.alloc(4).length,4);
});
