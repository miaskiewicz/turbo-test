//! Missing Buffer byte operations found by the unchanged Flux consumer campaign.
#![cfg(unix)]
mod common;
use common::{json_report, turbo_test, Project};
const CASES: &str = r#"
test('equals compares actual byte windows without changing subarray aliasing', () => {
 const source=Buffer.from([0,65,66,0]),view=source.subarray(1,3);
 expect(Buffer.isBuffer(view)).toBe(true);
 expect(view.equals(Buffer.from([65,66]))).toBe(true);
 expect(view.equals(new Uint8Array([65,66]))).toBe(true);
 expect(view.equals(Buffer.from([65]))).toBe(false);
 expect(view.equals(Buffer.from([65,67]))).toBe(false);
 expect(Buffer.alloc(0).equals(Buffer.alloc(0))).toBe(true);
 view[0]=77;expect(source[1]).toBe(77);
});
test('comparison is unsigned lexicographic across lengths and views', () => {
 const cases=[ [[],[],0], [[],[0],-1], [[0],[],1], [[255],[1],1], [[1,2],[1,3],-1], [[1],[1,0],-1], [[1,2],[1,2],0] ];
 for(const [a,b,result] of cases)expect(Buffer.compare(Buffer.from(a),new Uint8Array(b))).toBe(result);
 const view=new Uint8Array([99,1,2,99]).subarray(1,3);
 expect(Buffer.compare(view,Buffer.from([1,2]))).toBe(0);
});
test('comparison rejects non-byte inputs and spoofed byte brands', () => {
 for(const value of [null,[], 'a', new Uint16Array([1]),new DataView(new ArrayBuffer(1)),{[Symbol.toStringTag]:'Uint8Array'}]) {
  for(const operation of [()=>Buffer.compare(value,Buffer.alloc(0)),()=>Buffer.compare(Buffer.alloc(0),value),()=>Buffer.alloc(0).equals(value)]) {
   let error;try{operation()}catch(e){error=e}
   expect(error).toBeInstanceOf(TypeError);expect(error.code).toBe('ERR_INVALID_ARG_TYPE');
  }
 }
});
test('UInt32LE writes exact bytes, offsets, maximum and coerced values', () => {
 const b=Buffer.alloc(9,99);expect(b.writeUInt32LE(0x02014b50,1)).toBe(5);
 expect(Array.from(b)).toEqual([99,80,75,1,2,99,99,99,99]);
 expect(b.writeUInt32LE(0xffffffff,5)).toBe(9);
 expect(Array.from(b.subarray(5))).toEqual([255,255,255,255]);
 for(const [value,want] of [[1.9,1],['3',3],[NaN,0],[undefined,0]]) {
  const out=Buffer.alloc(4);expect(out.writeUInt32LE(value)).toBe(4);expect(Array.from(out)).toEqual([want,0,0,0]);
 }
});
test('UInt32LE invalid values and offsets throw without mutating bytes', () => {
 for(const [value,offset,code] of [[-1,0,'ERR_OUT_OF_RANGE'],[4294967296,0,'ERR_OUT_OF_RANGE'],[Infinity,0,'ERR_OUT_OF_RANGE'],[1,-1,'ERR_OUT_OF_RANGE'],[1,1,'ERR_OUT_OF_RANGE'],[1,1.5,'ERR_OUT_OF_RANGE'],[1,NaN,'ERR_OUT_OF_RANGE'],[1,'0','ERR_INVALID_ARG_TYPE'],[1,null,'ERR_INVALID_ARG_TYPE']]) {
  const b=Buffer.alloc(4,99);let error;try{b.writeUInt32LE(value,offset)}catch(e){error=e}
  expect(error.code).toBe(code);expect(Array.from(b)).toEqual([99,99,99,99]);
 }
 expect(()=>Buffer.alloc(4).writeUInt32LE(1n)).toThrow(TypeError);
 let error;try{Buffer.alloc(3).writeUInt32LE(1)}catch(e){error=e}
 expect(error.code).toBe('ERR_BUFFER_OUT_OF_BOUNDS');
});
"#;
#[test]
fn buffer_byte_comparison_and_unsigned_writes_match_node() {
    let p = Project::new("buffer-byteops-397-398-400");
    p.with_esbuild();
    p.file("package.json", "{}");
    p.file("jest.config.js", "module.exports={testEnvironment:'node'};");
    p.file("buffer.test.js", CASES);
    let run = turbo_test(&p.dir, &["--reporter", "json"], &[]);
    let report = json_report(&run);
    assert_eq!(report["numPassedTests"], 5, "{}", run.all());
    assert_eq!(report["numFailedTests"], 0, "{}", run.all());
}
