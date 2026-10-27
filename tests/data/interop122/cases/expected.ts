// vitest 4.1.12 (Node 26) on this project: identical under environment happy-dom and node,
// isolate true and false (turbo-test#122). Key: `<file>|<row>`.
export const expected: Record<string, string> = {
  "after-mock|exp-default-flag default":
    "fn:DefFlag",
  "after-mock|exp-default-flag named":
    "string:n",
  "after-mock|mexp-fn default":
    "fn:MexpFn",
  "after-mock|tsc default":
    "fn:TscComp",
  "dynamic|Comp":
    "fn:LessonPage",
  "dynamic|babel-out":
    "fn:BabelComp",
  "dynamic|barrel":
    "fn:LessonPage",
  "dynamic|esm-only ns":
    "{default=fn:EsmOnly,x=string:x}",
  "dynamic|exp-default-flag ns":
    "{default=fn:DefFlag,named=string:n}!esm",
  "dynamic|exp-default-flag":
    "fn:DefFlag",
  "dynamic|exp-default-noflag":
    "{default=fn:DefNoFlag,named=string:n}",
  "dynamic|exports-map":
    "fn:FromImport",
  "dynamic|mexp-fn ns":
    "{default=fn:MexpFn,module.exports=fn:MexpFn}",
  "dynamic|mexp-fn":
    "fn:MexpFn",
  "dynamic|reexports":
    "fn:DefFlag",
  "dynamic|require cjs-requires-esm":
    "{def=fn:EsmOnly,ns={__esModule=boolean:true,default=fn:EsmOnly,x=string:x}!esm}",
  "dynamic|require exp-default-flag":
    "{default=fn:DefFlag,named=string:n}!esm",
  "dynamic|require exports-map":
    "fn:FromRequire",
  "dynamic|require mexp-fn":
    "fn:MexpFn",
  "dynamic|tsc-out":
    "fn:TscComp",
  "mock|Comp":
    "{default=fn:LessonPage,readerSiteName=fn:readerSiteName}",
  "mock|automock default":
    "{Comp=fn:Comp,a=number:1}",
  "mock|automock named":
    "undefined:undefined",
  "mock|exp-default-flag":
    "{default=fn:DefFlag,named=string:n}!esm",
  "mock|exp-default-noflag":
    "{default={default=fn:DefNoFlag,named=string:n},module.exports={default=fn:DefNoFlag,named=string:n},named=string:n}",
  "mock|factory default":
    "fn:Mocked",
  "mock|factory without default":
    "throws:[vitest] No \"default\" export is defined on the \"exp-default-noflag\" mock. Did you forget to return it from \"vi.mock\"?",
  "mock|factory without default ===":
    "throws:[vitest] No \"default\" export is defined on the \"exp-default-noflag\" mock. Did you forget to return it from \"vi.mock\"?",
  "mock|factory without default shorthand":
    "throws:[vitest] No \"default\" export is defined on the \"exp-default-noflag\" mock. Did you forget to return it from \"vi.mock\"?",
  "mock|factory without default truthiness":
    "throws:[vitest] No \"default\" export is defined on the \"exp-default-noflag\" mock. Did you forget to return it from \"vi.mock\"?",
  "mock|factory without default typeof":
    "throws:[vitest] No \"default\" export is defined on the \"exp-default-noflag\" mock. Did you forget to return it from \"vi.mock\"?",
  "mock|importActual spread default":
    "fn:TscComp",
  "mock|importOriginal spread default":
    "fn:DefFlag",
  "mock|importOriginal spread named":
    "string:mocked",
  "mock|mexp-fn":
    "{default=fn:MexpFn,module.exports=fn:MexpFn}",
  "mock|tsc-out":
    "{default=fn:TscComp,named=string:n,viaDefault=fn:DefFlag,viaPlain=fn:MexpFn,viaStar={default=fn:MexpFn,length=number:0,name=string:MexpFn,prototype=obj}}!esm",
  "static|Comp default":
    "fn:LessonPage",
  "static|Comp named":
    "fn:readerSiteName",
  "static|babel default":
    "fn:BabelComp",
  "static|babel ns":
    "{default=fn:BabelComp,named=string:n,viaDefault=fn:DefFlag,viaPlain=fn:MexpFn,viaStar={default=fn:MexpFn}}!esm",
  "static|babel viaDefault":
    "fn:DefFlag",
  "static|babel viaPlain":
    "fn:MexpFn",
  "static|babel viaStar":
    "{default=fn:MexpFn}",
  "static|barrel LessonPage":
    "fn:LessonPage",
  "static|barrel default":
    "fn:LessonPage",
  "static|barrel ns":
    "{LessonPage=fn:LessonPage,default=fn:LessonPage,readerSiteName=fn:readerSiteName}",
  "static|barrel readerSiteName":
    "fn:readerSiteName",
  "static|cjs-requires-esm default":
    "{def=fn:EsmOnly,ns={__esModule=boolean:true,default=fn:EsmOnly,x=string:x}!esm}",
  "static|esm-import-meta default":
    "fn:MetaComp",
  "static|esm-import-meta urlType":
    "string:string",
  "static|esm-only default":
    "fn:EsmOnly",
  "static|esm-reexport-cjs default":
    "fn:MexpFn",
  "static|esm-reexport-cjs ns":
    "{Comp=fn:Comp,DefFlag={default=fn:DefFlag,named=string:n}!esm,a=number:1,default=fn:MexpFn,module.exports={Comp=fn:Comp,a=number:1}}",
  "static|exp-default-flag default":
    "fn:DefFlag",
  "static|exp-default-flag named":
    "string:n",
  "static|exp-default-flag ns.default":
    "fn:DefFlag",
  "static|exp-default-flag ns":
    "{default=fn:DefFlag,named=string:n}!esm",
  "static|exp-default-noflag default":
    "{default=fn:DefNoFlag,named=string:n}",
  "static|exp-default-noflag named":
    "string:n",
  "static|exp-flag-nodefault default":
    "{named=string:n}!esm",
  "static|exp-named default":
    "{Comp=fn:Comp,a=number:1}",
  "static|exp-named named":
    "string:number,function",
  "static|exports-map default":
    "fn:FromImport",
  "static|exports-map which":
    "string:import",
  "static|exports-map-default default":
    "fn:MapDefault",
  "static|local-flag.cjs default":
    "fn:LocalFlag",
  "static|local-flag.cjs named":
    "string:n",
  "static|local-noflag.cjs default":
    "fn:LocalNoFlag",
  "static|local-noflag.cjs named":
    "string:n",
  "static|local.cjs default":
    "fn:LocalCjs",
  "static|mexp-fn default":
    "fn:MexpFn",
  "static|mexp-fn ns.default":
    "fn:MexpFn",
  "static|mexp-fn ns":
    "{default=fn:MexpFn,module.exports=fn:MexpFn}",
  "static|mexp-has-default default":
    "{default=fn:Inner,named=string:n}",
  "static|mexp-obj default":
    "{Comp=fn:Comp,a=number:1}",
  "static|mexp-obj named":
    "string:number,function",
  "static|mexp-selfdefault default":
    "fn:SelfDefault",
  "static|mexp-selfdefault named":
    "string:n",
  "static|module-field default":
    "fn:MainCjs",
  "static|reexports Comp":
    "fn:Comp",
  "static|reexports DefNoFlag":
    "{default=fn:DefNoFlag,named=string:n}",
  "static|reexports MexpFn":
    "fn:MexpFn",
  "static|reexports default":
    "fn:DefFlag",
  "static|requires-exports-map default":
    "fn:FromRequire",
  "static|star-flag ns":
    "{named=string:n}",
  "static|star-mexp ns":
    "{module.exports={Comp=fn:Comp,a=number:1}}",
  "static|tsc default":
    "fn:TscComp",
  "static|tsc ns":
    "{default=fn:TscComp,named=string:n,viaDefault=fn:DefFlag,viaPlain=fn:MexpFn,viaStar={default=fn:MexpFn,length=number:0,name=string:MexpFn,prototype=obj}}!esm",
  "static|tsc reexport default":
    "fn:DefFlag",
  "static|tsc reexport ns":
    "{Comp=fn:Comp,a=number:1,default=fn:DefFlag}!esm",
  "static|tsc viaDefault":
    "fn:DefFlag",
  "static|tsc viaPlain":
    "fn:MexpFn",
  "static|tsc viaStar":
    "{default=fn:MexpFn,length=number:0,name=string:MexpFn,prototype={}}",
};
