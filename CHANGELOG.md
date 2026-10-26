# Changelog

All notable changes to `@miaskiewicz/turbo-test`. Format based on
[Keep a Changelog](https://keepachangelog.com/); this project uses semantic versioning.

## [Unreleased]

### Fixed

- SVG elements (and MathML ones, where they have their own interface) had no `style`: only
  `HTMLElement` carried the inline `CSSStyleDeclaration`, so `svg.style.flex = '1'` threw
  "Cannot set properties of undefined" — React sets inline styles on `<svg>` icons everywhere.
  `SVGElement.prototype.style` now returns the same live inline declaration (and `style = text`
  forwards to `cssText`), defined just before `attributeStyleMap` to keep Chrome's member order.
  Part of #79.

### Release procedure

- Determine missing V8 dependency versions through the official registry API,
  preserving errors for denied existing archives and exact payload checks.
- Add a manual, commit-guarded recovery for the immutable v0.4.7 tagged source.
  It publishes only the reviewed V8 dependency before the original tagged
  pipeline resumes; the product version, release tag and runtime stay unchanged.

## [0.4.8] - 2026-10-25 — Chrome-parity batch for turbo-surf#28

Closes a large set of measured missing DOM members and behaviours from
turbo-surf issue 28's paired API inventory. Every closed surface has an
independently captured Chrome 156.0.8078.47 / V8 15.6.75.19 control
(`src/*_chrome156.json`, probe SHA recorded) and an offline regression that runs
the same probe in the binding and asserts equality; irreducible deltas are pinned
as explicit KNOWN_GAP assertions, never skipped. No presence-only stubs: members
that cannot be truthful without a browser stay open. Integration PR: #29.

### Added
- HTMLMeterElement / HTMLProgressElement / HTMLOutputElement reflection, clamping,
  `htmlFor` token list and Chrome's output default-value/reset behaviour.
- HTMLSelectElement collection API (`add`/`remove`/`item`/`namedItem`/`length`/
  `size`/`type`/iterator, indexed access) and a live, same-object
  `HTMLOptionsCollection` for `select.options`; live `selectedOptions`.
- HTMLInputElement / HTMLTextAreaElement members (`stepUp`/`stepDown` with Blink's
  Decimal arithmetic, `valueAsDate`/`valueAsNumber`, `maxLength`/`minLength`,
  `size`/`cols`/`rows`/`wrap`, `indeterminate`, `list`, `textLength`, `type`, …)
  and Chrome value sanitization per input type.
- File-state inputs: `FileList`, `input.files`, `webkitEntries`, fakepath values,
  file form entries, and the `DataTransfer` / `DataTransferItemList` family.
- Invoker commands (`command`, `commandForElement`, `popoverTargetAction/Element`,
  `CommandEvent`) and genuine `HTMLDialogElement` behaviour (`show`/`showModal`/
  `close`/`requestClose`/`returnValue`/`closedBy`, events, `:modal`/`:open`,
  focusing steps, `method=dialog`).
- `getHTML`/`setHTMLUnsafe`, ShadowRoot init state (`clonable`/`serializable`/
  `delegatesFocus`), `activeElement`, `styleSheets`, `onslotchange`; declarative
  shadow DOM in the main document parse and `Document.parseHTML[Unsafe]`.
- HTML Sanitizer API and safe `Element`/`ShadowRoot` `setHTML`.
- `ShadowRoot.referenceTarget`, interest invokers (`interestForElement`,
  `InterestEvent`), and `focusgroup` arrow-key navigation.
- `document.all` (undetectable `HTMLAllCollection`), `domain`, `lastModified`,
  inert fullscreen / pointer-lock / picture-in-picture state.
- Pointer capture and pointer lock state machines driven by trusted input.
- Web Animations (`Element.animate`, `getAnimations`, `document.timeline`,
  `Animation`, `KeyframeEffect`, …) applied through computed style.
- CSS Typed OM Level 1 (`computedStyleMap`, `attributeStyleMap`, `styleMap`, all
  value classes) with a Chromium-156-generated property registry.
- CSS `zoom` grammar and computed value; `Element.pseudo()` / `CSSPseudoElement`;
  `Document.fragmentDirective`; `webkitMatchesSelector`; eight Element-level event
  handlers; CustomEvent/PopStateEvent/HashChangeEvent/ProgressEvent/CloseEvent;
  native link URL members on `<a>`/`<area>`.

### Fixed
- Strict CSSOM selector admission: unknown pseudo-classes are rejected like Chrome
  (forgiving `:is()`/`:where()` preserved); `querySelector`/`matches`/`closest`
  now throw `SyntaxError` for selectors Chrome rejects. The previously ignored
  unknown-pseudo regression now passes.
- Implicit inherited `color`/`visibility` see CSSOM rule mutations.
- Input-only members removed from the wrong control prototypes (HTMLLabelElement,
  HTMLSelectElement, HTMLTextAreaElement, HTMLOptionElement); control members
  enumerable as in Chrome.
- `document.activeElement` retargets out of shadow trees; `form.elements` works for
  forms inside shadow roots; a plain `new Event('click')` no longer runs checkbox
  activation; `<noscript>` serializes literally with scripting enabled.

### Known gaps
- Composed events dispatched inside a shadow tree still don't reach the host's
  ancestors; rusty_v8 `should_throw_on_error()` deltas on interceptors; prototype
  own-key order (`constructor` first). Tracked in turbo-surf issue 28.

## [0.4.7] - 2026-10-21

Release the combined browser ownership/lifecycle and computed-style follow-ups
from PRs #26 and #27. Publish the reviewed Rusty V8149.3.0 teardown fix as
`turbo-surf-v8` before the runner crate, preserving the upstream SDK version and
native archive URLs. Packaged consumers resolve the fork through a real package
alias rather than losing a root-workspace patch. Live DOM, forms, CSSOM and constructor controls retain their
independent Chrome evidence. Computed style reuses ancestor snapshots only within
one public read and skips declaration work for unmatched private selectors.
The unknown-pseudo CSSOM admission regression remains explicitly ignored with
Chrome's expected SyntaxError; its explicit expected failure is not counted as
passing conformance. Broader browser parity remains tracked in turbo-surf issue28.

- Match private style-rule selectors before reading declarations, avoiding full
  serialization of unmatched rules on every ancestor calculation. The captured
  native matcher receives complete selector lists, preserving quoted braces and
  commas; non-style-rule fallbacks and import/source order remain unchanged.
  Add work-count, live CSSOM and hostile-getter controls without a persistent
  computed-style cache. Preserve a separate Chrome-expected, explicitly ignored
  regression for preexisting unknown-pseudo CSSOM admission (turbo-surf issue28);
  its expected failure is recorded rather than treating acceptance as correct.
  Browser-specific strict/forgiving selector admission remains a separate feature.

- Reuse all resolved ancestor computed-style snapshots within one public read,
  including percentage lookups through auto-sized ancestors before font
  inheritance revisits an earlier node. Capture private WeakMap intrinsics at
  bootstrap and expire the cache before the next live declaration read. Bounded
  scan, mutation/reparenting and hostile-prototype controls prevent redundant
  ancestor work without introducing a persistent computed-style cache.

- Preserve parsed `!important` priority per computed longhand, including shorthand
  expansion and inline styles. Later normal declarations no longer overwrite
  important stylesheet values. Priority stays private; live stylesheet adoption,
  mutation and computed color/length normalization still apply. Include private
  document-adopted sheets after ordinary sheets without importing them into shadow
  scopes. The temporary adopted-sheet copy has no Array prototype, so numeric
  prototype setters cannot observe or alter this private collection. Genuine Chrome155
  controls cover the nine measured border precedence cases.

- Implement the measured constructible Text and DocumentFragment interfaces using
  captured native document factories. Preserve instance prototypes, constructor
  inheritance (including ShadowRoot), subclass `new.target` prototypes and owner
  documents, with lossless UTF-16 and Web IDL string conversion. Public factory
  poisoning does not intercept construction. Update stale compatibility checks
  against genuine Chrome constructor, shadow insertion and computed-color controls.

- Preserve Node constructor inheritance when installing the listener-backed
  EventTarget constructor; keep the original EventTarget instance prototype and
  native Node listener behavior intact. Add relational constructor/dispatch
  controls for the public interfaces.

- Seed measured Chrome155 button declarations at user-agent origin for computed
  style reads, including the normal Arial 13.3333px font reset, spacing and border
  widths. Author universal rules, inline values, explicit inheritance and initial
  font values take precedence; `revert` restores the button UA declaration.
  Source-bound independent Chrome controls cover default and inherited parent
  fonts, authored overrides and CSS-wide font keywords. Other form-control UA
  defaults and broader computed-value behavior remain outside this bounded fix.

- Normalize ASCII casing for the existing supported computed-font units and
  grammar keywords using a captured private lowercase intrinsic. Preserve quoted
  and custom font-family spelling and custom-property names. Native deferred-value
  admission now recognizes the same five CSS-wide keywords case-insensitively
  without accepting arbitrary identifiers. Additional independent Chrome155
  cases reproduce and prevent mixed-case units yielding `NaNpx`, uppercase
  shorthand misclassification and valid `INITIAL` resets being discarded.
  Preserve scalar `calc()` line-height as an inherited factor rather than
  misclassifying it as pixels. Unit-cancelling division remains unresolved.

- Resolve computed font longhands and shorthand from actual owner-document
  preferences and inherited values, including unitless versus absolute
  line-height, relative sizes, arithmetic `calc()`, CSS-wide keywords and native
  named/manual slot inheritance. Live declarations refresh after property-name
  conversion, retain receiver brand checks, and return empty font values after
  disconnection or browsing-context retirement. Independent Chrome155 controls
  cover fractional serialization, inert/template ownership, realm borrowing and
  adoption. Native embedders can set private scalar initial font preferences;
  macOS `Times` is measured, while the other-platform `Times New Roman` default
  remains uncertified. Metric-dependent units, viewport units, `min`/`max`/`clamp`,
  custom-property substitution and other per-element UA defaults remain outside this
  bounded computed-font resolver; unresolved text survives inheritance without
  fabricated zero or `undefined` values.

- Add a host-only custom-element context retirement boundary. Retained frame
  registries, definitions and existing instances stay usable, while automatic
  construction, upgrades and reactions stop for the retired definition realm,
  including already queued reactions and callbacks borrowed from an active
  realm. Fresh Chrome155 controls cover detached-frame creation, innerHTML,
  contextual fragments, explicit upgrades and existing-element adoption.

- Clear constructed adopted stylesheets on every migrated ShadowRoot during
  cross-document adoption, including nested closed roots, before custom-element
  adopted callbacks. Preserve retained ObservableArray identity, same-document
  adoption and disconnect/reconnect state, while bypassing poisoned public
  ownerDocument/adoptedStyleSheets getters and array methods. Fresh Chrome155
  controls cover unread roundtrips, foreign-realm sheets and assignment rejection.

- Share custom-element reaction queues across nested CEReactions frames. Advance
  the shared cursor before invoking a callback so reentrant adoption preserves
  already queued callbacks, per-element order, exception reporting and the
  original execution limits. Fresh Chrome155 controls cover callback realms,
  error receivers, two-element interleaving and nested attribute reactions.
- Parse Range contextual fragments through the public turbo-dom development
  revision `aaeb290`, using the actual post-conversion boundary context,
  namespace/integration attributes, form ancestry and document quirks mode.
  Preserve foreign and template-associated owner documents, scoped registries
  and inert nested templates without invoking a context-element constructor.
- Parse template innerHTML into a detached native fragment before inert-owner
  adoption, preventing premature custom-element construction and attribute
  callbacks. Add a bounded independent Chrome155 parser/observer lifecycle
  matrix with19 context cases, mutation notification/reentrancy controls and
  reusable `scripts/dom-lifecycle-chrome-control.mjs` collection. XML parsing
  and complete browser lifecycle parity remain separate work.

- Preserve LF checkout bytes for native JavaScript sources and JSON controls so
  independent Chrome source-hash assertions also verify on Windows. Licensed
  XPath provider bytes retain their explicit no-conversion exceptions.

- Reuse each private CSS rule record already resolved during import flattening
  instead of looking it up again immediately in computed-style evaluation.
  Preserve import/media traversal, source order and live retained declarations;
  nested-import edits and detached-rule mutations have regression coverage.

- Read already resolved parent computed values from their private backing records
  within one calculation, avoiding exponential live-getter recursion through
  inherited colors and percentage sizes. Retained external declarations still
  refresh on subsequent reads. Independent Chrome mutation controls and bounded
  stylesheet-scan regressions cover inheritance plus width/height chains.

- Refresh the private stylesheet-list backing array once per computed-style
  calculation instead of rescanning every style/link for each live-list index
  and length read. Retained computed declarations still recompute on every
  property access; independently measured Chrome controls cover CSSOM edits,
  stylesheet insertion/removal/replacement, disabled sheets and zero public
  document/style-list getter calls. No persistent
  computed-style cache or browser-conformance claim is introduced.

- Add genuine live, readonly `labels` NodeLists on Input, Button, TextArea,
  Select, Output, Meter and Progress. Native light-tree associations preserve
  first-ID targeting, null-namespace attributes, exact UTF-16 IDs,
  hidden-input transitions, shadow scopes,
  adoption and receiver-realm collection identity; share that authority with
  ElementInternals and label.control, including custom controls before builtins.
  Independent Chrome controls cover descriptors, wrong receivers, live mutations
  and hostile public methods. Production iframe/retirement GC validation remains
  a separate renderer gate with an explicit real-task collection policy.

- Make the adopted-radio GC control resolve readiness without retaining the
  load Event, clear its listener and record an explicit real-task GC policy.
  Preserve the original non-repeatable WeakRef receipt, three fresh original
  runs, three settled Chrome controls and actual native PageSession proof;
  retained radios keep their genuine owner until release without extra roots.


- Preserve an independent Chrome adversarial form-owner control for first-ID
  non-form/SVG/HTML-case collisions, open/closed shadow boundaries, disabled
  radio groups, external reset order, split moves and genuine XML documents.


- Index selected radios by native node and prior native form owner, without
  JavaScript roots. Normalize checked descendants and batch fragment order on
  container moves, keep shadow-tree form lookup within its actual root, and
  reassociate external radios after form ID/removal changes. Route the seven
  existing control `.form` getters through genuine native ownership with readonly
  per-interface brands and descriptors. Preserve detached
  unowned radio behavior, genuine checked-accessor brands, cloning/adoption and
  native storage reclamation with independent Chrome controls. Empty indexes
  skip descendant traversal on ordinary non-radio moves.


- Keep radio grouping in the native checkedness intrinsic, including checked/name/
  type/form attribute changes, parsed subtrees, directly moved radios, and form reset. Restore
  the prior selected radio after canceled clicks without guest checked hooks.
  Preserve native checkedness overrides across actual cross-document adoption;
  source-pinned Chrome controls cover dirty defaults, cloning, live old/new form
  collections, cancellation and forced-GC observations.


- Add genuine live HTMLFormControlsCollection and RadioNodeList interfaces, form
  elements/length/iteration, owner-aware external/custom controls and cached named
  groups. Private records support borrowed base accessors across real V8 contexts,
  preserve measured named/index ordering and mutation behavior, and release owner
  realms after retained collections are dropped. Independent Chrome controls cover
  descriptor shapes, live membership, radio values, collisions and DOMString errors.
  Broader radio dirty/default/reset/adoption lifecycle remains active follow-up;
  genuine production iframe/retirement controls are supplied for integration gates.

- Expose genuine Element.customElementRegistry with native receiver checks and
  private creation/parser associations. Preserve scoped/null registries across
  moves; global associations follow adoption to the new owner document without
  retaining the former global registry; inert-null adoption rebinds correctly,
  including unread descendants, ShadowRoots and multi-hop moves. Parse-completion
  hooks record ownership before any public getter read, without enabling plain-DOM
  reaction recording.
  Independent Chrome controls cover unread parser nodes, adjacent HTML, inert
  documents, cross-realm borrowing/adoption and property descriptors.

- Reflect Input/Button formAction, formEnctype, formMethod and formTarget on their
  own prototypes with native owner-document/base URL resolution, per-interface
  string conversion, missing/invalid enum defaults and genuine receiver checks.
  Independent Chrome controls include malformed URLs, attribute writes, borrowed
  accessors, wrong namespace/local-name receivers and hostile public getters.

- Match computed styles through captured genuine native selector and owner-document
  intrinsics. Page overrides of matches/ownerDocument are not invoked internally;
  live stylesheet mutations remain visible without new cascade caching.

- Keep descriptor rules out of nested cssRules lists so the private native CSSHost
  snapshot retains @property/@view-transition descriptor bodies. Independent Chrome
  shape controls and poisoned-public-getter export regressions cover the boundary.

- Validate complete @property and @view-transition descriptor blocks before
  checking rule shells. Preserve validated registration descriptors and typed
  navigation/types instead of dropping them as ordinary CSS declarations. This
  retains CSSOM source; it does not implement property registration semantics.

- Make the genuine ValidityState constructor prototype read-only, matching Chrome;
  extend the independent form control oracle with descriptor and native brand checks.

- Match Input/Button own prototype placement, enumerable control descriptors and
  text-selection conversion/errors; setRangeText performs real replacement and
  selection updates rather than an inert compatibility method.

- Add measured native HTMLFormElement reflected attributes and live relList,
  including action URL resolution from the host-installed document URL, enum
  normalization, encoding alias, USVString action conversion and branded accessors.
  Independent Chrome controls include relative/base and malformed URL serialization.

- Suppress native form submission for disconnected forms before validation/events
  and recheck connection after submit listeners, using the captured native Node
  accessor. Independent Chrome controls cover removal and reconnection timing.

- Add branded ElementInternals and CustomStateSet with native custom-element
  admission, form ownership/validation, live labels, default ARIA and closed-shadow
  access. Association, disabled and reset callbacks use native reaction queues.
  Private V8 records preserve cross-realm brands and object-owned lifetimes.
- Match native `:state()` selectors with exact UTF-16 membership, fresh clone/import
  state and adoption migration. Branded setlike iterators resume after exhaustion
  when new entries appear and retain their actual owner without host roots.
- Add genuine FormDataEvent identity/descriptor behavior and host-only trusted
  dispatch plus guarded form-entry construction transactions. Runtime adapters
  provide real private File/FormData snapshots; a standalone binding supplies no
  networking backend. Native FileList and radio grouping remain follow-up work. Dirty checkbox state, reset, clone and typed :checked now use native records.
- Pin the public turbo-dom development revision
  `fb60e6f90a5d373165cfd1045b090407ca945d47` for native custom-state selectors;
  this does not imply registry publication of its 0.5.7 candidate.

- Preserve FormData optional argument conversion and current-document authority
  when no form is supplied; reject invalid HTMLElement submitters before the
  empty result. Add a native base-target authority accessor for navigation adapters.

- Expose a host-only native form entry-list constructor for private FormData
  adapters, preserving submitter validation, duplicate ordering and UTF-8 hidden
  charset values without dispatching validation or submit events.

- Capture an optional host-only File/FormData value reader through document-owned
  private V8 edges, so runtime adapters can provide genuine immutable snapshots
  without trusting guest object shape or public metadata properties.

- Add native-document-owned submission admission and queue callbacks with immutable
  ordered text entry snapshots, submitter overrides and hidden charset-entry metadata.
  Preserve validation/cancellation before transport; native FileList and full select
  state support remain explicit follow-up work. Independent Chrome155 FormData
  controls cover duplicate names, disabled/checked controls, text, charset and submitter.

- Replace unconditional form-validity answers and no-op custom validity with live
  native-DOM control state, branded ValidityState objects, required/custom/type/
  pattern/numeric checks, trusted non-bubbling invalid events, and form validation.
  Add a branded SubmitEvent and requestSubmit argument/admission/validation/event
  behavior, preserving native submitter and Chrome155 composed-event differences.
  Capture live control values and native accessors instead of reading guest overrides;
  form-owner lookup includes explicit form IDs. Uncanceled navigation still needs a
  consumer transport and reports a clear NotSupportedError instead of pretending
  to submit; click default-action failures are reported as engine exceptions.

- Implement Chrome155's AbstractRange → NodeRange → Range/StaticRange interface
  hierarchy with native descriptors, constructor inheritance and genuine private
  static boundaries. Static offsets retain their original values through native
  tree mutations; getters accept authentic foreign-realm records and reject forged
  or proxied receivers. Match sorted Web IDL dictionary conversion, unsigned offsets,
  invalid node types, subclassing and poisoned public intrinsic controls. Keep live
  Range mutation tracking unchanged. Real iframe retirement is a separate consumer gate.

- Preserve quoted commas, comment-looking strings and escaped commas when CSSOM
  serializes selector preludes. A lexical scan normalizes only actual separators;
  fresh Chrome155 controls cover parsing, selectorText changes and cssText reloads.

- Add a host-only per-document form-submission admission flag for sandboxed
  consumers. Block the submit default action after an uncanceled control click
  while preserving the click event; keep policy isolated across native documents.
  The consumer supplies sandbox/CSP intersection; this does not implement form
  navigation or destination authorization.

- Add native private-state Trusted Types factory, policy and value brands, exact
  UTF-16/USVString conversion, callback conversion and Chrome155 interface descriptors.
  Cross-realm values retain their native brands without host roots. Authored Chrome
  controls cover sink-type mappings and conversion errors. Native HTML, script,
  attribute and Range sinks invoke immutable document-owned host authorization;
  additive exports let consumers enforce parser/timer sinks and supply exact private
  TrustedScript source to the engine without guest coercion. Production CSP and
  eval/timer execution remain responsibilities of the consumer runtime.

- Implement DocumentFragment.getElementById, inherited by ShadowRoot, through
  iterative native tree-scope lookup. Preserve exact UTF-16 IDs, first-match order,
  empty/missing behavior and native conversion/receiver errors without invoking
  replaceable query/getAttribute methods. Extend the independent installed Chrome155
  factory control with fragment/closed-shadow lookup, intrinsic-poisoning and ID changes;
  add deep lookup and scalar-repair collision regressions.

- Claim pending native mutation batches across all documents before invoking resource
  consumers. A consumer's nested query can no longer drain another document's old
  removal events into its inner custom-element reaction scope, so cross-realm adoption
  callbacks observe the final ownerDocument. Add a regression with querying consumers
  in both realms against the unchanged independent Chrome155 lifecycle control;
  genuine nested mutations, interruption and GC contracts remain covered.

- Implement genuine CustomElementRegistry and HTML element constructors over native nodes:
  autonomous/customized construction, late upgrade, live attribute/connect/disconnect/adopt
  callbacks, scoped registry lookup/initialization, clone/import behavior and trusted construction
  error reporting.
- Add native per-element custom-element reaction queues, including moveBefore connectedMoveCallback
  and fallback lifecycle ordering; native RAII scopes recover from interrupted constructors,
  definitions and callbacks without retaining page realms.
- Replace isolate-rooted JavaScript collection-record maps/functions with context-free V8 Private
  keys and per-object records; unused installed document contexts, custom-element nodes and scoped
  shadow cycles now collect. Preserve cross-realm brands, frozen/proxy record support and
  interruption behavior.
- Add six fresh, source-hashed normal Chrome155/V8 .23 control pairs plus native
  GC/termination/private-payload regressions. Form-associated ElementInternals lifecycle remains
  follow-up work; these tests establish the measured contracts rather than full browser conformance.

- Consume verified published `turbo-dom` 0.5.6 for parser provenance and native
  template extraction; the dependency resolves from crates.io with its published
  checksum, without a private DOM path override.

- Preserve the native Window `onstorage` listener slot when combining parser
  provenance and inert-template prerequisites. The handler keeps branded
  descriptors, listener ordering, replacement and non-callable clearing; its
  standalone regression is retained from the frozen storage-events checkpoint.

- Preserve parser-time style-attribute CSP checks inside genuine template contents.
  Native preorder follows the private template graph and passes the parsing
  document's policy without activating inert stylesheets or inventing source
  positions. The combined standalone cohort passes 214 contracts; the downstream
  Chrome control retains its document-targeted report for a template attribute.

- Combine the reviewed real associated inert template implementation with native
  parser provenance and original-unit CSS callbacks. Template contents use their
  actual separate document, so active queries and stylesheet authorization do
  not cross into them. Imported inert storage conservatively has no new lexical
  source token. The combined standalone cohort passes214 contracts; the additive
  backend dependency is published in turbo-dom0.5.6.

- Preserve authoritative original UTF-16 code units in native stylesheet and
  style-attribute authorization callbacks. Scalar policy compatibility views
  remain separate. A native regression covers independent lone high/low units
  in both sources, supporting exact CSP report-sample delivery downstream.

- Add opt-in original UTF-16 parser source positions for host installation and
  private native node/attribute provenance. CSS authorization receives trusted
  parser metadata and actual Window ownership. Inert template styles remain
  unprepared until active adoption; initial and dynamic authorization stay
  distinct. Native controls cover CRLF/non-BMP positions, illegal objects,
  poisoned guest document access and template activation. Requires the additive
  turbo-dom source-position API; full runtime report ingress is tested separately.

- Implement HTMLScriptElement.text through native direct child Text/CDATA code
  units and the existing native text replacement/preparation pipeline. Fresh
  Chrome controls cover brands, descriptors, coercion, original units and actual
  execution on connection or filling an empty connected script. Failed conversion
  preserves nodes, Range boundaries and observer queues. The merged binding passes
  179 standalone contracts; Trusted Types policy enforcement remains pending.

- Consume published turbo-dom 0.5.5 and preserve original UTF-16 payloads, names,
  namespaces, attributes, mutation records, parser input and inline handler/script
  source through native two-byte V8 strings. Scalar compatibility views remain at
  existing host boundaries. Independent document installation and iterative script
  preparation cover real ownership and 8192-level host-stack regressions. Fresh
  Chrome controls cover surrogate identity, BOMs, lookup/reflection, cloning and
  source text. The merged standalone binding passes 177 native contracts; Surf
  runtime module/frame source integration remains a separate pending gate.

- Expose host-only native composed children/parent identities for layout. Closed
  roots and manual assignment use private records; hidden fallback and unslotted
  children have no composed parent. A native regression poisons public slot APIs
  and preserves authentic internal projection without exposing closed roots.

- Implement native named/manual shadow-slot distribution, live assignedSlot,
  fallback flattening and trusted slotchange in the shared observer notification
  microtask. Original wrapper edges preserve foreign adoption and manual stealing;
  clearing an assignment releases its old slot. Source-pinned Chrome controls cover
  brands, dictionaries, closed roots, light slot elements and reentrant ordering.
  Actual V8 collection and a 4096-level iterative fallback walk guard retention
  and native stack behavior. The complete binding cohort passes 165 tests.

- Reflect HTMLIFrameElement.credentialless through genuine HTML iframe brands and captured native attribute operations; boolean conversion, descriptors, poisoned public methods and illegal receivers match a fresh Chrome 155 control.

- Add the native-branded, live iframe sandbox DOMTokenList with Chrome155 token
  support. Token-list PutForwards setters honor own value setters and failed
  ordinary assignment; source-pinned Chrome controls cover identity, descriptors
  and HTML namespace/local-name receiver behavior.

- Reflect HTMLMetaElement name/httpEquiv/content/media/scheme through native attributes, with genuine HTML namespace/receiver brands, DOMString conversion and private native method capture. Saved normal Chrome155 descriptor/property/attribute/illegal-receiver controls cover the complete narrow surface.

### Fixed
- Dispose cached native worlds safely after binding thread-local cells have been
  destroyed. Availability guards preserve normal isolate restoration while the
  isolate-owned saved state still clears before V8 handle invalidation. A real
  thread-exit regression covers the cached-runtime initialization/drop order.
- Partition native document, listener, observer, CSS and focus state by its actual
  V8 isolate. Scoped activation restores the previous isolate/document after
  nested work, cold installation and disposal; resetting or dropping one isolate
  preserves its peers. Eight regressions cover identity, independent observers,
  reset/disposal ordering and activation guards that outlive their isolate.
- Consume published turbo-dom 0.5.4 bounded storage collection through its public
  API. Genuine weak wrappers, private ownership edges and host-job node pins keep
  reachable nodes alive; successful collection also removes stale scalar records.
  Thirteen contracts cover retained/adopted nodes, pending mutations, job pins,
  budgets and teardown. Collection is explicit; automatic host-boundary scheduling
  remains consumer work, and multi-document calls are atomic per Tree.
- Resolve computed colors through the public cascade serialization and private
  linked-sheet values. Cover inherited/default CanvasText, CSS-wide keywords,
  currentColor, disconnected elements and live declarations against a fresh
  source-pinned Chrome 155 control. Broader system-color/color-scheme, variables
  and layer cascade behavior remains open.
- The combined canonical browser binding cohort passes 158 contracts on the
  default 149.3.0 SDK. Consumer runtime scheduling and genuine native155 checks
  remain separate; inherited runner warnings prevent claiming a strict full-repo
  gate from this focused result.
- Select HTML interfaces by exact namespace/local name rather than uppercasing
  XML-created names. Preserve genuine HTMLUnknownElement versus valid custom-name
  identity, reserved names and legacy generic/preformatted aliases. Control getters
  and methods check native namespace/local-name brands even when borrowed or when
  public prototypes are changed. A fresh 723-row authored Chrome155 matrix covers
  these HTML/null-namespace mappings and foreign control brands; SVGScriptElement
  and SVGImageElement projection/resource behavior remain separate open gaps.
- Allocate native ownership component/helper maps in the actual Document realm.
  A foreign script getter cannot retain its retired Window through another
  Document’s map shape. Observer event draining no longer dirties unchanged graphs.
  Two new regressions bring the current canonical cohort to 134 passing contracts.
- Store script parser/force-async/already-started flags as native scalar node state.
  Preserve clone/import/adoption and inert/retired-document behavior; prepare native
  candidates after observer fanout, including nested and shadow-root scripts. Six
  new contracts and a fresh Chrome155 control cover this subset. The integrated
  canonical cohort passes 132 tests; runtime fetch/execution wiring, parser reentry,
  SVG scripts, import maps and speculation rules remain open.
- Keep Rusty V8 weak handles live through native slot/finalizer disposal using a
  source-pinned, reproducible 149.3.0 dependency patch. Preserve its real SDK/native
  engine identity and upstream license; the original teardown regression fails,
  while 16 patched isolates and all 126 canonical binding contracts pass.
- Make private helper publication fallible during iframe budget interruption,
  preserving pending V8 termination and leaving interrupted helpers unpublished.
  Normal construction and a canceled-interrupt retry remain covered.
- Register inert document initializers on their callback creation realm. Shared
  embedder ops no longer overwrite the main Window initializer with the last iframe
  closure; a heap-traced retained-Document regression identified that native root.
- Pin the captured RegExp exec intrinsic on private XML-name validation, so replacing
  RegExp.prototype.exec cannot intercept native document factories. Validate doctype
  delimiters directly; the expanded Chrome155 control covers seven poisoned intrinsics.
- Replace native wrapper/helper/context roots with weak lookups and private
  Document/component ownership. Keep genuine receiver and Node-argument owners
  pinned during native callbacks, preserve ordinary exceptions and materialized
  V8 termination, and invalidate interrupted wrapper publication before retry.
  The combined canonical ownership/observer checkpoint passes 124 contracts;
  dependency disposal regressions are tracked separately from that binding gate.
- Deliver native MutationObserver records through one V8 notification microtask,
  with per-observer options, old values, namespace filtering, transient removed
  subtrees, static branded NodeLists and receiver checks. Compound insert/replace
  algorithms emit their semantic batches, preserving ordered source removals and
  failed cross-document conversion. Saved Chrome 155 controls and hostile builtin
  regressions cover this subset. Add a trusted Document-private mutation consumer
  for resource postconnection work, with observer fanout first and nested mutations.
  Broader lifecycle, shadow slots, custom elements and UTF-16 work remain open.
- Retain the native node projector privately on its owner Document, avoiding
  page-replaced constructor and public projector getters during first wrapping,
  including parser-created controls and independent documents. Reject inherited
  tag-map entries and keep interrupted wrapping out of the identity cache. The
  canonical browser binding cohort passes 119 contracts.
- Preserve XML element case and document-specific default namespaces for native
  createElement, including borrowed receivers and colon-containing XML names.
  Keep createElement local names distinct from createElementNS qualified names;
  saved Chrome controls cover HTML, XML, XHTML and SVG document factories.
- Create genuine independent HTML/XML document trees through branded
  DOMImplementation factories and Document constructors. Preserve native doctype
  adoption identity, title conversion, receiver checks and foreign stylesheet
  rejection, without page-replaced RegExp/String/Array/Map/Object hooks. Cover
  ordinary parsed documents without factory metadata as well as the saved Chrome
  155 factory control. XML parsing, CDATA and downstream XML helpers remain open.
- Capture native node projection methods so first-time wrapping of Text, Attr,
  Comment, elements and fragments cannot call page-replaced Function, WeakSet,
  Object, Array or String methods. Cover this alongside repeated document installs
  and the XPath private-intrinsic regression.
- Consume published turbo-dom 0.5.3, including its additive mutation-event metadata
  for observer delivery; browser observer queues remain a separate integration task.
- Preserve native Node traversal/equality descriptors when rebuilding a document
  in an existing realm; project compatibility delegates from that bootstrap's
  actual raw node prototype rather than a previous constructor's prototype.
- Adopt genuine foreign nodes during all native insertion methods after validating
  the full destination sequence. Preserve cached node/descendant identity, listener
  state and source Range adjustment. Single-node insertion validates references
  and Document hierarchies before changing an owner or parent. DocumentFragment and ShadowRoot
  insertion moves their children while keeping the original container and owner;
  repeated arguments retain their observed ordering. An independent Chrome 155
  collector covers borrowed methods, namespace data and single-node failure state.
- Match Chrome155 variadic conversion before destination checks: drain fragments,
  detach multi-node sequences, retain the last duplicate and resolve viable sibling
  references. Failed compound insertion leaves converted nodes detached in their
  source document and updates source Ranges; failed WebIDL conversion has no such
  effects. A shared authored Chrome fixture covers failure and fragment overlap.
- Expose the real empty, stable NodeList from Attr.childNodes with native receiver
  branding. Include Attr and CDATA kinds in the Node receiver predicate; the HTML
  binding still rejects createCDATASection and does not add an XML parser.
- Make binding cleanup safe during thread-local runtime destruction regardless
  of initialization order. Clear every remaining private handle cache while
  treating already-destroyed TLS cells as released, including diagnostic storage.
- Keep live Range boundaries synchronized with insertAdjacentHTML, outerHTML,
  innerText and outerText replacement. Match Chrome's parent-offset collapse when
  outerText appends and removes adjacent text, instead of applying normalize's
  boundary-transfer behavior. The exact shared mutation collector and saved Chrome
  155 control cover all four insertion positions and empty/multiline replacements.
- Read stylesheet text, media and title through captured native DOM accessors,
  preserving actual sheet identity across CSSOM edits despite page-defined getters.
  Add a Rust-only CSSHost live export with private rule/declaration state, disabled
  and media values, grouping and attached import provenance; bypass public getters,
  iterators and poisoned prototypes with explicit graph/text budgets. Correct
  font-face descriptor validation, mutable declaration edits and URL serialization.
  Seven offline contracts include an independent installed Chrome155.0.8059.79 control;
  this does not implement layout, font loading or complete CSS interface shapes.
- Expose native-only generic EventTarget listener observation for embedding lifetime
  management, using genuine private listener state and the creation realm. Cover
  add/remove/once/abort cleanup, foreign targets and forged receivers without guest
  getters or page-visible registration capabilities.
- Update live Range boundaries through private weak native records for text edits,
  UTF-16 offsets, splitting, insertion, removal, bulk child replacement, normalization,
  state-preserving moves and node adoption. Match an independent Chrome155.0.8059.79
  control without guest DOM getter/method hooks; cover foreign trees and actual V8
  garbage collection. Geometry, remaining parsing/text mutation seams and lossless
  unpaired UTF-16 storage remain open.
- Implement native-tree Range clone/extract/delete/surround content operations,
  real selected text, boundary ordering/offset/root validation, node intersections
  and atomic insertion preflight. Preserve extracted node identity and nested
  partial shells, genuine receiver checks and private final exception constructors.
  Saved Chrome155 content/error controls and hostile DOM-method regressions cover
  this subset; layout geometry and remaining mutation-seam coverage remain open.
- Build DOM collection snapshots, private event paths and returned composed-path
  copies through intrinsic own elements, bypassing guest numeric setters,
  constructors/species and iterators while retaining live/native node identities.
- Propagate native Node events through the actual owner Document and Window with capture,
  target and bubble phases, stable composed paths, stop/removal/once/passive contracts and
  immediate AbortSignal subscriptions. Nonbubbling resource errors reach Window capture;
  element load events exclude Window. Preserve inert-document and shadow-root boundaries,
  foreign event/signal brands, listener objects and native handler registration order.
  Window onerror receives genuine ErrorEvent's five source/error arguments and return-true
  cancellation; ordinary on* handlers receive one Event and cancel on return false.
  Capture final trusted ErrorEvent/AbortSignal interfaces and the default AbortError
  DOMException constructor after host environment installation. Expose dispatch records through
  a Rust-only native_event_record seam with genuine/foreign/forged Event brand checks.
  Generic EventTarget/subclass and AbortSignal listeners also use private native records:
  no public registry fields, native receiver/foreign brands, real onabort handlers and
  once/passive/signal/removal/stop semantics. Match measured registration order at direct
  Window/generic/Signal targets while preserving Node/Document capture and bubble phases.
  Create private listener/dispatch records without prototypes, append/copy/remove private
  arrays through own indices without guest setters/species/iterators, and allocate native
  facade data/construct arguments with intrinsic array creation. Expose Rust-only platform
  brands and an idempotent opaque host target initializer in the actual creation realm.
  Hostile index/species/iterator/state-setter controls retain callbacks with zero hooks.
  Native Node abort-hook data and Window ErrorEvent argument arrays also bypass inherited
  numeric setters during subscription and onerror dispatch.
  Complete shadow retargeting/composed host crossing and listener exception reporting remain
  explicit followups.
- Implement the HTML-standard legacy Document clear/captureEvents/releaseEvents no-effect
  methods with native Document receivers, descriptors and ignored-argument behavior.
- Implement Document XPath evaluation/compiled expressions and XPathEvaluator over bounded
  public-rtdom snapshots using the unmodified MIT xpath 0.0.35 provider in a private V8
  context. Preserve real Node/Attr identity, namespace and foreign-case matching, corrected
  following/preceding axes, scalar/snapshot/iterator contracts, mutation invalidation,
  opaque native brands and owner-realm results for borrowed foreign interfaces. Enforce
  explicit node/depth/string/expression/operation budgets; reject namespace-axis results
  and fragment/shadow contexts. Keep the provider's automatic page installation disabled.
- Capture popover task schedulers by the current native context document after host timer
  installation, preserving independent frame queues and the parent's private scheduler.
- Preserve createElement's actual Document receiver through control/iframe enrichment, with
  single tag coercion, native ownership for inert documents, and IDL/name validation. Reflect
  script/link integrity properties into loader attributes with native receiver checks. Add
  legacy createNSResolver as validated Node identity backed by native namespace resolution.

## [0.4.6] - 2026-10-13

- Implement stable ObservableArray-backed stylesheet adoption for genuine Documents
  and ShadowRoots. Private backing preserves ownership, conversion/error effects,
  indexed/deletion/descriptor behavior and native-sheet constructor-document checks.
  Renderer exports read captured private backing, bypassing page-owned adoption
  getters; independent Chrome 155 mutation/ownership controls cover this subset.

### Fixed
- Back popover show/hide/toggle with native per-document state, auto/hint/manual ancestry
  and source relationships, cancellable trusted beforetoggle events and coalesced trusted
  toggle tasks on a privately captured host scheduler. Keep live :popover-open selector
  composition and shadow-root separation without changing DOM attributes or styles. Handle
  ordinary removal/moves, state-preserving moveBefore, null-namespace attribute changes,
  inert documents and queued event identity across adoption. Preserve assigned on* source
  markers during cross-document adoption. Autofocus/restoration, light-dismiss/backdrop,
  fullscreen/modal integration, stateful CSS cascade and top-layer painting remain pending.
- Validate native DOM insertion before changing parent links: reject ordinary and shadow-host
  ancestor cycles, enforce final Document element/doctype/text constraints, preserve nodes on
  invalid insertions, and check native Node brands, references and removal membership. Route
  variadic, adjacent and replacement operations through the shared validator; preserve valid
  fragment transfer, repeated-node ordering, string insertion and self-reference behavior.
  Native hierarchy errors retain genuine DOMException identity through private constructors.
- Use published turbo-dom 0.5.2 as the canonical DOM dependency. Add native HTML innerText/outerText
  mutation and supported rendered-text flow, nullable popover reflection and branded ToggleEvent
  records; complete CSS layout and top-layer painting remain unsupported.
- Keep native `classList` and `part` token-list identity and live indexed state,
  validate mutations atomically, and reflect ARIA element references with private
  assignment state, attribute invalidation and shadow-tree scope checks. Add native
  `moveBefore` hierarchy/root checks and preserve node/focus/control/handler identity;
  complete Range/iterator/slot/custom-element move reactions remain unsupported.
- Compile authored HTML event-handler attributes as real V8 functions with native element,
  form and document scope; keep assigned-function overrides and host CSP authorization private.
- Initiate inline stylesheet imports through private host state and include imported declarations
  in live computed styles; cache native policy filtering by the actual tree mutation version.
- Create native ProcessingInstruction nodes with CharacterData inheritance, XML-name/data
  validation, serialization, identity-preserving adoption and clone/import support. CharacterData
  methods now use UTF-16 offsets and reject invalid ranges; HTML CDATA creation throws the
  measured NotSupportedError. Preserve actual supported HTML XML-metadata state.
- Apply host CSS authorization without changing authored DOM attributes; preserve parser policy
  position and separate CSP-blocked attribute text from applied declarations and CSSOM writes.
  Keep stylesheet loader capabilities private and guard opaque stylesheet rules/mutations.
- Share CSS interface records across same-origin V8 realms, retaining borrowed getter and method
  behavior while rejecting unbranded receivers.
- Preserve element and attribute namespace URIs, prefixes and qualified names across native
  creation, mutation, cloning and import; keep native Attr identity, live values and single-owner
  attachment semantics, with branded live NamedNodeMap access and namespace validation.
- Back CSSStyleSheet, StyleSheet and MediaList interfaces with private records and native branded
  callbacks; retain live lists, owner/import metadata and media mutations, detach removed rules,
  reject invalid single-rule insertions, and filter imports from constructed replacements.
- Share CSSStyleDeclaration native prototype callbacks across private inline, rule and
  read-only computed records; synchronize inline mutations and external attribute edits,
  retain stable identity and validate supported properties with the stylesheet parser.
- Reflect editing and accessibility attributes with native branded accessors, including enum
  normalization, hidden union conversion, spellcheck/translation inheritance, associated-form
  auto-capitalization/correction and draggable image/link defaults.
- Isolate DOM trees, native node ownership, style state and listeners by V8 context.
- Separate input/textarea live values from defaults; retain reflected hidden values, default select
  selection and intentional empty selections.
- Validate CSS selectors and supported property values with Lightning CSS, recover malformed rules,
  and retain strings, custom properties, grouping rules, and deferred `var()`/`env()` values.
- Represent fetched linked stylesheets in document order, preserve their URL and import rules,
  and detach stylesheet ownership when their link leaves the document.
- Expose CSSRuleList and StyleSheetList as branded indexed collections rather than arrays;
  rule-list identity survives replacement and the document stylesheet list tracks tree changes.
- Separate native Node/Element/HTMLElement/Document and character-data interface prototypes;
  shared native accessors remain live and reject invalid receivers. Document-specific instance
  properties and compatibility APIs still have partial Web IDL coverage.
- Normalize Event constructor required arguments, string types and optional initialization,
  including CustomEvent's default detail.
- Back CSS list methods/accessors and DOM interface constructors with native V8 callbacks;
  preserve constructor inheritance, read-only prototype properties and non-enumerable globals.
  Rebuilding a document refreshes interface constructors without losing native tree bindings.
- Preserve class construction, subclass `new.target`, live instance state and exceptions through
  the native callable bridge used by host runtimes.
- Create genuine native document fragments, flatten their children on insertion, and retain
  fragment clone/query identity. Native shadow roots preserve host/mode and scoped tree queries.
- Put Node constants, DOM event methods and form-control methods on their owning interfaces;
  retain native DOM listeners and subclass EventTarget state without exposing cascade helpers.
- Keep event state private, expose read-only event fields and distinguish host-generated trusted
  dispatch from script dispatch across DOM, custom EventTarget and window listeners.
- Preserve CSS list receiver identity across document replacement and same-origin frame access;
  new style elements use the current document's CSSOM closures.
- Project supported document collections and creation methods onto Document; normalize text nodes,
  replace children and compare actual native tree positions through Node methods.
- Store element event-handler properties privately and invoke them during native event dispatch;
  preserve measured legacy Window load/pageshow state and real DOMException errors.
- Read HTML/SVG/MathML namespace identity from the native tree, retain foreign-element case and
  namespaces during cloning, and resolve native namespace prefixes through ancestor elements.
- Reflect scalar ARIA and basic HTML attributes through native node storage; expose attribute
  enumeration and adjacent text insertion on Element, and move link/script/control properties
  onto their HTML interfaces without copying them onto unrelated elements.
- Put style/import rule fields behind native, branded prototype accessors with private state;
  selector/style setters update serialization, and import ownership uses an internal loader seam.

- Add live native indexed HTMLCollection/NodeList views over document collections and native
  children, with static querySelectorAll results and private cross-realm receiver records.
- Preserve native node/Attr/shadow identities, ownership, listeners and control/nonce state
  during adoption; clone/import supported private live values through captured document helpers.
- Read DocumentType metadata and compatibility mode from the real HTML parser; implement
  supported Document scalar reflection, handler ordering, nonce state and tabIndex conversion.

## [0.4.5] — 2026-10-10

### Fixed
- Parse style-element and constructed stylesheet rule lists, retain CSSStyleRule identity and selectors,
  refresh after text mutations, and track only connected style elements in document.styleSheets.
- Replacing style text creates a new stylesheet and detaches the previous sheet while retaining its rules.
- Parser-created form controls now receive their interface prototypes and populated IDL values.
- Frame construction accepts a host-provided window factory for engines with independent V8 contexts.

## [0.4.4] — hide internal window state

### Changed
- Move internal iframe/event state to the shared Symbol namespace.

## [0.4.3] — bump turbo-dom to 0.5.1 (faster cascade)

### Performance
- Upgraded the native DOM engine `turbo-dom` `0.4.0` → `0.5.1`. The headline win is a cascade
  fix: `getComputedStyle` no longer re-parses every `<style>` sheet on each DOM mutation. The
  parsed rule index used to be keyed on the DOM version (bumped by *any* mutation), so a React
  re-render — which restructures nodes but leaves the injected CSS untouched — forced a full
  re-parse of all stylesheets on the next `getComputedStyle`. `turbo-dom` 0.5.1 keys the index on
  the `<style>` sources, reusing it across version bumps whose CSS is unchanged. Measured ~3–5%
  faster wall on a 457-file / 7324-test React component suite, identical pass counts.
- Also picks up 0.5.0's rtdom internal-consistency fixes (stale parent pointers, unset event
  target) — neither reachable through `query_selector*`/serialize, so no behavior change here.

## [0.4.2] — browser_env: iframe realm, SwiftShader WebGL, content-dependent canvas

Fidelity additions to the rtdom↔V8 `browser_env` DOM binding (consumed by turbo-surf's
browserless render tier for its reCAPTCHA/anti-bot work):
- **Bridged second-realm `<iframe>`** — `contentWindow`/`contentDocument` is a real
  second window realm with two-way cross-realm `postMessage` (the channel a reCAPTCHA
  anchor↔bframe handshake needs).
- **SwiftShader WebGL context** — `getContext('webgl'|'webgl2')` returns a coherent
  headless-Chrome ANGLE/Vulkan-SwiftShader signature (getParameter incl. UNMASKED_*,
  ~40 limits, extension list, getShaderPrecisionFormat, deterministic readback);
  previously `null`.
- **Content-dependent canvas2d readback** — `toDataURL`/`toBlob`/`getImageData` now
  hash the recorded draw ops (deterministic, non-empty) instead of a constant stub.
## [0.4.1] — memoize per-file config/project walks

### Performance
- **Per-file project/config walks are now memoized (thread-local, per start directory).** Five
  pure-of-the-filesystem functions in the module-load hot path — `cjs_first_project`,
  `vitest_setup_files`, `project_root`, `nearest_pkg_type`, and `is_esm_module` — each walk up the
  directory tree re-reading `package.json` and up to ~11 config filenames per ancestor. Their result
  depends only on the *directory* chain (never the filename — except `is_esm_module`, which also
  keys on the file extension), yet they were recomputed for every test file and, for the
  per-module ones, for every module a file imports. Each now caches into a `thread_local` `HashMap`
  keyed by the start dir, exactly like the existing E12 `nearest_tsconfig` / `resolve_spec_as`
  memos and gated behind the same `TURBO_NO_E12` flag. Keying by the start directory (rather than
  the file path) lets sibling test files and co-located modules share one answer, turning repeat
  walks into hashmap hits. The syscalls saved are the config `is_file()`/`read_to_string` and
  `node_modules/.bin/esbuild` `exists()` probes that dominate warm-cache runs once transforms are
  already cached.
- **`cache_dir()` no longer calls `create_dir_all` on every invocation.** It is hit ~9× per module
  load; the directory only needs creating once. The creation is now guarded by a process-wide
  `OnceLock<Mutex<HashSet<PathBuf>>>` so the syscall fires exactly once per distinct cache dir
  (the set keys on the resolved path so a `TURBO_CACHE_DIR` override is still honored).

  Behavior is unchanged — identical resolution results, identical test outcomes. Measured on real
  suites (warm cache, ABBA-interleaved medians to cancel thermal/load drift; `cargo build --release`
  native binary, 12 jobs, Apple Silicon):
  - **payroll-app** (1583 files / 18418 tests): runner wall ~15.48s → ~15.19s (**~1.9% faster**).
  - **ui-design-components** (453 files / 7268 tests): ~12.48s → ~12.35s (**~1%**).
  - **website-global** (101 / 1476) and **standalone-chat-app** (68 / 761): within noise (walks
    are a tiny fraction of these short runs).
  - Cold-cache runs are transform-bound (esbuild dominates) and land within noise — never a
    regression. All four suites reported identical pass counts before and after.

## [0.4.0] — window self-reference + native plugin/alias support

### Fixed
- **A top-level window did not self-reference (issue #18).** `window.parent`, `window.top`, and
  `window.self` were all `undefined`, so `window.parent !== window` — the canonical "am I inside an
  iframe?" check — read `true` for a non-framed window. Real browsers, jsdom, and vitest all return
  `=== window` for a top-level document, so any component gated on top-level/iframe detection (e.g. a
  CMS live-preview hook that only mounts inside the preview iframe) behaved as if *always framed*
  under turbo-test, forcing tests to stub the framing globals by hand. The window object
  (`window === globalThis`) now defines `self`/`parent`/`top`/`frames` pointing at itself and
  `frameElement = null`. They're `configurable`, so a test can still stub framing explicitly
  (`Object.defineProperty(window, 'parent', { value: iframeWin })`) to exercise the framed path.

### Added
- **`resolve.alias` / `test.alias` from the vitest config are resolved natively.** Both the object
  form (`{ '@': path.resolve(__dirname, './src') }`) and the array form
  (`[{ find, replacement }]`) are parsed by the launcher and fed to the module resolver, so a prefix
  alias like `@` → `src` turns `@/foo` into `src/foo` while leaving `@scope/pkg` node_modules imports
  alone. The alias target is the last string literal in the value expression (covers `path.resolve`,
  `fileURLToPath(new URL(…))`, and bare literals); regex `find` keys are skipped. This is the
  non-tsconfig counterpart to the tsconfig `paths` aliases turbo-test already resolves.
- **`vite-plugin-svgr` (`?react`).** `import Icon from './x.svg?react'` now resolves to a
  render-safe React component — a dependency-free function component that returns a real `<svg>`
  React element with the props forwarded (className / data-testid / aria-* pass through), plus the
  legacy `ReactComponent` named export. Plain `import url from './x.svg'` still yields the file
  contents. Other Vite query suffixes (`?url`, `?raw`, …) no longer hard-error — they resolve the
  base file.
- **Documented native support** for the most common vitest plugins (`vite-tsconfig-paths`,
  `@vitejs/plugin-react`(`-swc`), `@vitest/coverage-v8`, `@testing-library/jest-dom`,
  `vitest-canvas-mock`) in the README — most suites need no plugin setup.
- **More vitest CLI flags accepted** instead of warned: `--run`, `--watch`/`-w`, `--pool`, `--mode`,
  `--project`, `--exclude`, `--maxConcurrency`, `--sequence.*`, `--logHeapUsage`,
  `--hideSkippedTests`, `--disableConsoleIntercept`, `--inspect*`, `--browser*` (accepted-and-ignored
  on a native single-run runner), and `--no-file-parallelism` (→ runs files serially, `jobs = 1`).
  Value-taking forms no longer leak their argument as a bogus test-file path.

### Changed
- **Reporter output gained a vitest-style summary footer** (`Test Files` / `Tests` / `Duration`) for
  the human reporters (default / dot / verbose), matching vitest's recognizable end-of-run block. The
  existing per-file `PASS`/`FAIL` lines and the turbo diagnostic line are unchanged; machine
  reporters (json / junit / tap) keep clean stdout.

## [0.3.15] — functional Web Streams + `expectTypeOf` on the vitest surface

### Fixed
- **`ReadableStream`/`WritableStream`/`TransformStream` were present but non-functional (issue #15).**
  The globals existed — so libraries that do `class X extends TransformStream` at module load kept
  loading — but the reader's `read()` unconditionally resolved `{ done: true }`, so any code that
  actually *consumed* a stream (fetch response-body streaming, SSE parsing) silently saw an empty
  stream. Replaced the stub with a working implementation: a chunk queue with `pull` backpressure,
  reader/writer locking, `cancel`/`abort` propagation to the underlying source/sink, `tee()`,
  async iteration (`for await`), and `pipeTo`/`pipeThrough`. The guard still defers to a host that
  already provides native streams. Not spec-exhaustive (no BYOB readers, no queuing-strategy
  `size`), but matches Node's observable behavior for the common consumer patterns.

### Added
- **`expectTypeOf` and `assertType` on the `vitest` compatibility surface (issue #16).** They were
  missing from both the runtime module shim (`import { expectTypeOf } from 'vitest'` threw
  `does not provide an export named 'expectTypeOf'`, and the namespace-member form failed at
  runtime as `not a function`) and the bundled type shim (so type-level assertions didn't
  type-check either). Type assertions are erased at compile time and have no runtime behavior in
  vitest, so the runtime is a chainable no-op (a proxy that answers every member with itself, with
  `then` excluded so an accidental `await` can't hang). The checking lives in the `.d.ts`:
  `ExpectTypeOf<Actual, Positive>` implements `toEqualTypeOf`/`toMatchTypeOf`/`toHaveProperty`/
  `toBe*`/`.not`/navigation (`items`/`returns`/`resolves`/…), where a mismatch surfaces as a
  compile error. Exported from `types/vitest.d.ts` and declared as globals in `types/globals.d.ts`,
  alongside `describe`/`it`/`expect`/`vi`.

### Tests
- `fixtures/compat/streams.test.ts` (15 cases) — the issue-#15 minimal repro plus lazy `pull`,
  async `start`, async iteration, `error`/`cancel` propagation, `tee`, subclassed
  `TransformStream`, and `pipeTo`/`pipeThrough`; asserted via the JSON reporter in
  `test/compat-api.test.mjs`.
- `fixtures/compat/expecttypeof.test.ts` (5 cases) — `expectTypeOf`/`assertType` resolve and run
  through named, namespace, and global access.
- `types/typetests/expecttypeof.test-d.ts` — compile-only guard (run under `tsc --strict` by
  `test/compat-types.test.mjs`) covering positive assertions and `@ts-expect-error` negative cases
  so the shim fails in both directions.

## [0.3.14] — setupFiles matchers no longer lost for a whole worker under reuse/coverage

### Fixed
- **`setupFiles`' `expect.extend` matchers (jest-dom's `toBeInTheDocument`, `toHaveTextContent`,
  `toBeDisabled`, …) intermittently missing for an entire worker's chunk of files under the
  isolate-reuse path — and under `--coverage` even with `--isolate`.** Symptom in the consuming app
  (payroll-app CI shards): a random subset of `.tsx` tests failing `expect(...).toBeInTheDocument is
  not a function`, bursty per-worker (a broken worker cascades its whole file chunk), unreproducible
  in isolation. `--isolate` masked the reuse trigger but coverage stayed flaky, so there was no CI
  workaround.
- **Root cause:** `esbuild_bundle_full` (used to build the setup-file bundle) wrote the output
  straight to the shared content-addressed cache path via esbuild's `--outfile`, which streams the
  file **non-atomically**. The cache-hit check is a bare `out.exists()`, so when many workers raced
  to create the *same* setup bundle at startup, a worker could read a **half-written or empty**
  bundle. An empty setup bundle imports nothing → `expect.extend` never runs → that worker's single
  reused `__customMatchers` registry stays empty for **every** file it owns (the whole-worker
  cascade). `run_setup_file` swallowed the resulting load, so the failure was silent. Every *other*
  esbuild cache path already wrote through `write_atomic` (temp + rename) for exactly this reason;
  this one path did not. It now builds to a unique temp file (applying the mock-rewrite pass there)
  and **atomically renames** into the cache — a racing worker sees either no file or the complete
  one, never a partial. Fixes both the reuse and coverage triggers (both go through this bundle).
- The `fresh-isolate retry` was auto-healing most of the reuse failures (re-running setup per file
  in a clean isolate), which is why the reuse trigger was flaky rather than constant and why it hid
  under `--isolate` — but it re-ran whole files at a cost, and could not help the coverage path.

### Added
- `TURBO_CACHE_DIR` env var to override the shared transform/bundle cache location — lets a test run
  against an isolated, freshly-cleared cache without disturbing other concurrent turbo-test processes
  (used by the new regression test).

### Tests
- `fixtures/jestdom-reuse/` — a vendored `jest-dom-ish` package whose top-level side-effect is
  `expect.extend({...})`, a setupFile that imports it, `isolate: false`, and 30 files that each call
  the custom matcher. `test/compat-reuse-matchers.test.mjs` runs it under reuse against a clean
  isolated cache (forcing the concurrent first-build) and asserts all matcher assertions run + pass.

## [0.3.13] — EventTarget is now a real subclassable base (React hydration)

### Fixed
- **`class Foo extends EventTarget` now works.** The ctor shim gave `EventTarget` an
  empty prototype, so `this.addEventListener(…)` inside any subclass (event buses,
  state stores, modal managers — Nike's global-nav modal manager is one) threw
  `addEventListener is not a function`. An uncaught throw mid-hydration makes React
  discard the SSR tree and client-render the whole root (#425), blanking
  server-rendered content. `EventTarget.prototype` now carries a working
  `addEventListener`/`removeEventListener`/`dispatchEvent` listener registry, so
  subclass instances behave as event targets and hydration completes over the SSR DOM.

## [0.3.12] — DOM binding: hasChildNodes/namedItem + form-control methods (React hydration)

### Added
- **`Node.hasChildNodes()`** on the native element binding. It was absent, so the
  NON_MASKING interceptor returned `undefined` and React's hydration/commit path threw
  `hasChildNodes is not a function` — aborting the client-render fallback.
- **`namedItem` on the array-like NodeList/HTMLCollection/NamedNodeMap** returned by
  `children`/`childNodes`/`querySelectorAll`/`getElementsBy*`/`attributes` (bound next
  to the existing `.item`). Libs calling `namedItem` no longer hit `not a function`.
- **Form-control methods** `setCustomValidity`, `setSelectionRange`, `setRangeText`,
  `select`, `scrollTo`, `scrollBy` (no-ops), plus `checkValidity`/`reportValidity`
  (always valid). Missing methods threw mid-hydration, making React discard the SSR
  tree and client-render the whole root (#425), losing server-rendered content.

## [0.3.7] — node-env jest: decorator-metadata via tsc + interface design:type partial

### Fixed
- **`emitDecoratorMetadata` files now route through the project's TypeScript on first load** — on
  both the CJS and ESM load paths — emitting `Object` for non-value types (ts-jest parity for local
  type aliases / nullable types) instead of oxc's identifier-based metadata. Fixes a decorated field
  typed as a **local** type alias and the nullable-metadata cases.
- **The routing now actually fires on ts-jest backends.** It required `project_root`, which only
  resolves a dir containing `node_modules/.bin/esbuild`; projects that ship `typescript` but NOT
  esbuild (the common ts-jest setup) hit `None` and silently fell back to oxc. New `tsc_root` walks
  up for `node_modules/typescript`, so tsc runs. Net on a real NestJS+Sequelize suite (flux-apis):
  **+79 passing tests** (decorated files whose metadata tsc lowers correctly now load).

### Known limit
- A decorated field typed as an **imported interface** (`@Column() declare x: SomeImportedIface`)
  still fails ESM instantiation (`does not provide an export named SomeImportedIface`). Neither oxc
  nor `ts.transpileModule` has a cross-file type checker, so both keep the type-only import and
  reference it in `design:type`; the interface has no runtime export. Fix is app-side
  `import type { … }` (elided by any transform) or a full TS program (too heavy for a runner).
  ~73 such cases remain on flux-apis.

## [0.3.6] — node-env jest: gated decorators + `.`/`..` re-exports + native-addon crash guard

Makes a real NestJS + Sequelize/Mongoose backend (flux-apis) load and run under turbo-test, from
two background spikes. On a 120-spec subset, clean-loading files went **30 → 53** and
decorator-syntax / TDZ load-errors **83 → 0**; and a native `.node` addon `require()` no longer
takes down the whole run.

### Fixed
- **TypeScript legacy decorators + `emitDecoratorMetadata` are lowered in the native transform —
  gated, on both load paths.** The oxc TS-strip pass used `TransformOptions::default()`, which
  doesn't lower *legacy* decorators, so a NestJS/Sequelize/Mongoose decorated class emitted either
  `export @Decorator class X {}` (→ `Unexpected token 'export'`) or oxc's 2022-standard lowering
  that re-reads the class binding mid-init (→ `Cannot access 'X' before initialization` TDZ) — and
  because both fail at *run* time, the esbuild retry-on-load was never taken → hard load-error. New
  `transform::transform_with` / `esm_cjs::emit_with` lower legacy decorators + metadata (babel
  helpers are global via `runtime.js`), called from both `read_transformed` (ESM) and
  `native_transform_cjs` (CJS), **gated** on `experimentalDecorators`/`emitDecoratorMetadata` being
  enabled AND the file actually using a decorator AND not being in `node_modules`, with the flag
  folded into the transform cache keys — so non-decorated files pay nothing (no blanket
  metadata-pass cost). The pre-existing CJS circular machinery (installing `module.exports` before
  the body runs) was already correct; decorators were the actual cause of the "circular init" TDZ.
- **`require(".")` / `require("..")` directory re-exports resolve.** NestJS packages
  (`@nestjs/sequelize`, `config`, `mapped-types`, …) chain `__export(require("./dist"))` and
  `dist/utils/x.js → require("..")` self-references. `bundler::is_relative` excluded bare `.`/`..`,
  so they were left as runtime externals resolved against the bundle's package-root dir →
  "cannot resolve ..". `is_relative` now accepts `.`/`..`; and `resolve_spec_as` falls through to
  index probing when oxc resolves such a spec to a *directory* instead of its index file.
- **A native `.node` N-API addon no longer SIGSEGVs the whole run.** turbo-test's host
  (`src/napi_host.rs`) exported ~37 napi symbols; a real napi-rs addon (e.g. `turbo-html2pdf`) calls
  ~14 more, and under macOS flat-namespace `-export_dynamic` those undefined symbols resolve to
  `0x0`, so the addon's first such call jumps to NULL (exit 139, zero output, every other test in
  the run killed). Implemented the safe missing symbols (buffers, array/element/property-name
  access, bool/double/int64 getters); routed genuinely-unsupported entrypoints
  (`napi_wrap`/`napi_unwrap`/`napi_new_instance`) to a **catchable JS throw**; and wrapped addon
  module-init + every native callback FFI call in `catch_unwind` — so a broken/unsupported addon
  fails its own file cleanly and the rest of the run continues.
- **`__dirname` / `__filename` on the ESM graph load path.** CJS modules get them as wrapper
  params; a real ES module (the `load_graph` path) had neither in scope, so an app `.ts` that reads
  `const DIR = __dirname` at top level threw `ReferenceError: __dirname is not defined` and the file
  failed to load. `inject_dirname_esm` now prepends Node-CJS-equivalent module-local `const`
  bindings, gated on actual reference + no local declaration, newline-free (line numbers/source maps
  preserved). (`browser_env` projects are unaffected — only files reaching `__dirname` change.)

### Tests
- `fixtures/compat/circular/` (plain ESM require cycle) and `fixtures/compat/circular-decorators/`
  (decorated, mutually-referencing models + local tsconfig) — both fail on the pre-fix binary, pass
  with the fix — wired into `test/compat-api.test.mjs`.
- `fixtures/napi/` (a synthetic misbehaving C addon, built at test time, `.node` gitignored) +
  `test/compat-napi.test.mjs`: asserts the run does not segfault (`signal !== 'SIGSEGV'`,
  `code !== 139`) and a sibling spec still executes after the bad-addon file.

### Known follow-ups (not here)
- A separate, pre-existing native-bundler bug: it can't resolve `tslib` inside
  `@aws-crypto/crc32c` (cascading through `@aws-sdk/client-s3`) — the bulk of the remaining
  flux-apis load-errors, unrelated to decorators/circular init.

## [0.3.5] — `.d.ts` shim round 4: adopt vitest's canonical types

Stops the `it.each` whack-a-mole by lifting vitest's real type definitions verbatim (read from
`@vitest/runner` / `@vitest/spy`) instead of hand-approximating, and audits the rest of the shim
against the upstream source.

### Fixed
- **`it.each` typing replaced with vitest's canonical overload set** (`ExtractEachCallbackArgs` +
  `EachFunctionReturn` + the four `TestEachFunction` overloads, verbatim from `@vitest/runner`).
  Tuple rows up to 10 columns (const or not) map to precise positional args; ragged / heterogeneous
  / >10-column rows land on the permissive `T[]` / `fallback` overload with array args — exactly the
  runner's behavior. Fixes the last residual (payroll-app `it.each` `1 → 0`) and supersedes the
  hand-rolled `Writable<T>` overloads from 0.3.4.
- **Test bodies and hooks now receive a context arg.** vitest's `TestFunction` is
  `(context) => …` and `beforeEach`/`afterEach`/`beforeAll`/`afterAll` callbacks receive
  `(context, suite)`; the shim typed them as zero-arg, so `it('x', ({ expect }) => …)` /
  `beforeEach((ctx) => …)` failed to assign (a callback with more params than the target is not
  assignable). Added a permissive `TestContext` (typed `task`/`signal`/`expect`/`skip`/`annotate`/
  `onTestFailed`/`onTestFinished` + an index signature) and widened `TestFunction` / `HookFunction`
  to pass it. No-arg bodies still assign.
- **`ExpectStatic` gained an index signature** so forward-compat statics (`expect.poll`,
  `expect.unreachable`, `expect.assertType`, `expect.addSnapshotSerializer`, …) resolve instead of
  erroring — matching the shim's permissive-subset philosophy.

### Tests
- `types/typetests/each.test-d.ts` extended: ragged non-const `it.each` rows (fallback path),
  context-receiving test bodies + destructured context, hooks with/without a context arg, and the
  forward-compat `expect` statics. Still gated by `test/compat-types.test.mjs` (`tsc --strict`).

## [0.3.4] — `.d.ts` shim round 3 (`it.each` `as const` rows) + constructable stylesheets

### Added
- **Constructable stylesheets** in the DOM bootstrap: `new CSSStyleSheet()` (with
  `cssRules`/`rules`, `insertRule` returning the index, `deleteRule`, `replace` → `Promise<this>`,
  `replaceSync`) plus a real, settable `document.adoptedStyleSheets` array. emotion/MUI do
  `new CSSStyleSheet()` then `document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet]`;
  there's no layout engine, so the rule list is an inert store that satisfies the API — but a
  missing `CSSStyleSheet` reference threw mid-chunk during hydration and blanked the tree.
  (`browser_env.js`.)

### Fixed
- **`it.each([...] as const)` regression.** The 0.3.3 tuple overload constrained rows to mutable
  `any[]`, so `[…] as const` (readonly tuples) fell through to the single-value overload and the
  multi-arg callback's params went implicit-`any` (payroll-app: `it.each` errors `2 → 30`). The
  spread overload now (1) accepts `readonly` rows via `ReadonlyArray<readonly [...T]>` to force
  per-position tuple inference (non-const heterogeneous rows + 1-tuples included), and (2) adds a
  second overload matching the union-of-readonly-tuples that `as const` produces, mapping it to
  mutable positional params with a `Writable<T>` helper so the spread assigns. `it.each` /
  `test.each` / `describe.each` and their `.only`/`.skip` variants all share `TestEachFunction`,
  so the fix covers every form.

### Tests
- `types/typetests/each.test-d.ts` — a compile-only guard exercising the it.each tuple/`as const`/
  union/scalar rows plus the rounds 1–2 surface (`vi.mocked`/`hoisted`/`fn<T>`, mock
  `importOriginal<T>`, `expect.fail`). Run by `test/compat-types.test.mjs` via `tsc --noEmit
  --strict` (new `typescript` devDependency; skips if absent). A shim regression now fails `npm test`.

## [0.3.3] — `.d.ts` shim round 2 (mock `importOriginal` generic + tuple-preserving `it.each`)

### Fixed
Round-2 type widening: after 0.3.2, payroll-app's `tsc` errors were `525 → 18` (+1 unrelated PNG).
The remaining 18 trace to two more type-only shim gaps in `types/turbo-test-api.d.ts`:
- **`importOriginal` is now generic** in the `vi.mock` / `vi.doMock` async-factory overloads — the
  factory's first arg is typed `<T extends Record<string, any>>() => Promise<T>` (vitest's
  signature), so `const actual = await importOriginal<typeof Mod>()` resolves and the spread /
  derived-`Parameters` usage downstream stops cascading off `{}`. (~16 errors: TS2347 ×11, TS2698,
  TS2339 ×4)
- **`it.each([...])` preserves per-position tuple types.** The 0.3.2 array overload widened each row
  to an element-union, so `it.each([['x', true]])(…, (label, isLast) => …)` saw
  `string | boolean`. The tuple overload now takes `ReadonlyArray<readonly [...T]>`, forcing each
  row to infer as a tuple → `(label: string, isLast: boolean)`. Ordered before the single-value
  overload; numeric-tuple spread and the explicit-union single-arg form (gap 5) still resolve. (2)

## [0.3.2] — extra HTML*Element constructor globals

### Fixed
- Register the HTML element constructors the DOM bootstrap's base ctor list omitted:
  `HTMLDialogElement`, `HTMLDataListElement`, `HTMLFieldSetElement`, `HTMLLegendElement`,
  `HTMLOListElement`/`HTMLDListElement`, `HTMLPreElement`, `HTMLTableRowElement`/`-CellElement`/
  `-SectionElement`/`-ColElement`/`-CaptionElement`, `HTMLProgressElement`, `HTMLMeterElement`,
  `HTMLDetailsElement`, `HTMLPictureElement`, `HTMLSourceElement`, `HTMLMediaElement`/
  `HTMLVideoElement`/`HTMLAudioElement`, `HTMLTemplateElement`, `HTMLSlotElement`,
  `HTMLBodyElement`/`HTMLHtmlElement`/`HTMLHeadElement`, `HTMLMetaElement`, `HTMLLinkElement`,
  `HTMLTitleElement`, `HTMLBaseElement`, `HTMLBRElement`, `HTMLHRElement`, `HTMLOptGroupElement`,
  `HTMLMapElement`, `HTMLAreaElement`, `HTMLObjectElement`, `HTMLEmbedElement`,
  `HTMLOutputElement`, `HTMLQuoteElement`, `HTMLMenuElement`, `HTMLDataElement`, `HTMLTimeElement`,
  `HTMLUnknownElement`. App bundles reference these for feature-detect / `instanceof` /
  subclassing (e.g. MUI's Dialog touches `HTMLDialogElement`); an undefined reference threw mid-
  chunk during hydration and blanked the rendered tree. Single-tag elements get a tag-keyed
  `instanceof` (`x instanceof HTMLDialogElement` ⇔ `x.tagName === 'DIALOG'`); abstract/multi-tag
  interfaces fall back to a generic element check. The native `get_constructor` tagName→name map
  is extended in lockstep so `node.constructor.name` resolves to the specific interface, not a
  generic `HTMLElement`. (`browser_env.js` bootstrap + `browser_env.rs`.)

### Fixed — `.d.ts` shim widened to match real vitest API usage
The 0.3.1 type subset was too narrow to replace `vitest`'s types in a large suite (payroll-app:
525 `tsc` errors across ~170 spec files with `vitest` dropped). Widened `types/turbo-test-api.d.ts`:
- **`vi.mocked()`** now returns a `MockedFunction<T>` for functions (exposing `mockReturnValue`/
  `mockReturnValueOnce`/`mockResolved|RejectedValue(Once)`/`mockImplementation(Once)`/
  `mockClear|Reset|Restore` and `.mock.{calls,results,instances}`) and a `MockedObject<T>` for
  objects/`{deep:true}` — the dominant define-then-configure pattern. (~350 errors)
- **`vi.hoisted<T>(factory: () => T): T`** added to `ViAPI`. (92 errors)
- **Hook callbacks** (`beforeEach`/`afterEach`/…) widened to vitest's `() => unknown | Promise<…>`,
  so `beforeEach(() => vi.useFakeTimers())` (arrow returns the chainable `ViAPI`) type-checks. (81)
- **`Mock<T>` / `vi.fn<T>()`** adopt the vitest-4 function-signature generic
  (`Mock<(a: string) => number>`, `vi.fn<(a) => b>()`) instead of the old args-tuple generic. (54)
- **`it.each<T>()`** gains a single-value-row overload so an explicit non-array union type arg
  (`it.each<InviteType>(['a','b'])`) resolves; tuple rows still spread into positional args.
- **`expect.fail(message?)`** added to `ExpectStatic`. (1)

## [0.3.1] — turbo-dom 0.3.5 + jest `@jest/globals` imports + shipped TypeScript types

### Fixed
- Bump the `turbo-dom` crate (rtdom DOM runtime) `0.3.4` → `0.3.5`. The new release stops
  `rtdom::serialize` from escaping regular spaces to `&nbsp;` (U+00A0) — a quirk inherited from
  the old JS serializer. That corruption made a re-parse / accessible-name / text query see
  `"a b"` instead of `"a b"`, so whitespace-normalized matching
  (`getByRole`/`getByText`/`toHaveText` against a serialized DOM) failed. Spaces now serialize
  verbatim; only `&`/`<`/`>`/`"` are escaped.

### Added
- **`import { … } from '@jest/globals'`** now resolves from turbo-test's own runtime, the same
  builtin that backs `import … from 'vitest'`. jest projects with `injectGlobals: false` (or
  TS-strict setups that import the API explicitly) previously fell through to node-module
  resolution and broke. The builtin shim now also exports `jest` (alongside
  `describe`/`it`/`test`/`expect`/`vi`/hooks/`assert`), so the named `jest` import binds to the
  runtime controller. (`runner.rs`: `is_test_api_module()` matches both specifiers across the
  static-ESM, dynamic-`import()`, and CJS-`require` resolution paths.)
- **Shipped `.d.ts` bundle** so consumers can drop the `vitest` (or `@types/jest`) devDependency
  that was kept purely for types. `tsconfig.json` `types: ["vitest/globals"]` and `from 'vitest'`
  imports across a codebase otherwise force `vitest` into devDependencies even when turbo-test is
  the runtime. New `types/`: `globals.d.ts` (ambient globals → `@miaskiewicz/turbo-test/globals`),
  `vitest.d.ts` (the `vitest` module), `jest-globals.d.ts` (the `@jest/globals` module), over a
  shared `turbo-test-api.d.ts`. A pragmatic vitest/jest-compatible **subset** (matcher/mock args
  widen to `any` — no false type errors, less precise inference). Exposed via `package.json`
  `exports` (`./globals`, `./types/*`) and added to `files`. README → "TypeScript types".

### Tests
- `fixtures/jest/src/jest-globals-import.spec.ts` + `test/compat-jest.test.mjs` cover the
  `@jest/globals` named-import path alongside the existing global-`jest` shim fixture.

## [0.3.0] — all-Rust DOM is the default (JS turbo-dom removed)

The DOM environment is now turbo-dom's pure-Rust **rtdom**, bound natively to V8. The legacy JS
`installGlobals` bootstrap + the `.node` parser + the `@miaskiewicz/turbo-dom` npm dependency are
**gone**; `TURBO_RUST_DOM` is no longer consulted (rtdom is unconditional). All three production
oracles run 100% green with **zero env flags** (payroll 10,471/0, ui-design 7,062/0,
website-global 1,003/0).

### Changed
- **Flip:** `browser_env::enabled()` is always true; `setup_dom` binds rtdom directly. Removed
  `dom_bootstrap`/`turbodom_root` (imported `install.mjs` + shimmed CSSOM) and dropped the
  `@miaskiewicz/turbo-dom` runtime dep. esbuild stays (coverage / decorator-metadata / fallback).

### Added — all-Rust DOM coverage (rtdom + browser_env binding)
- DOM event-dispatch fix: the V8 NON_MASKING name interceptor defers to a real own property before
  returning undefined — V8's inline cache could otherwise mask React's `__reactFiber$`/`__reactProps$`
  expandos (added after a cached miss), breaking delegated onClick/onChange on portal'd content
  (MUI Autocomplete × userEvent). Cleared the whole cluster + the `--jobs 8` flakiness.
- Real Selection/Range (live caret, `getRangeAt`, `setBaseAndExtent`, `selectionchange`) + native
  CharacterData (`insertData`/`splitText`/…) so contenteditable editors (Lexical) + userEvent typing
  work; `contentEditable`/`isContentEditable`; visibility resolved via rtdom's inheritance-aware
  native cascade; `input.valueAsNumber`.
- Form-control reflection; doc state (`readyState`/`visibilityState`/`elementFromPoint`/
  `getClientRects`); native ChildNode/ParentNode (`before`/`after`/`replaceWith`/`replaceChildren`);
  `insertAdjacentHTML`/`Element`; `toggleAttribute`; `getAttributeNS`; `setAttributeNode`/
  `removeAttributeNode`; anchor URL decomposition; `link.rel`/`media`/`as`/`type`; `localName`.
  (rtdom DOM methods land in the published `turbo-dom` crate 0.3.4.)
- `testTimeout` honored from the vitest config (was a fixed 5000ms default).

### Fixed — runtime
- `URL`: `search`/`href`/`toString` derive live from `searchParams`, so post-construction
  `searchParams.set()` serializes (was frozen at construction). `URLSearchParams` form-urlencoded
  space ⇄ `+`. `MessageEvent` global added.
- Config scan: read test `include`/`exclude` only from the text before the `coverage` block — a
  `coverage.exclude` whose first glob is `**/*.test.{ts,tsx}` was wrongly taken as the test exclude
  → "no test files found".

### Added — Rust port (branch `rust-port`)
- **P1: launcher ported into the native binary (`src/launcher.rs`).** Default test discovery,
  vitest config include/exclude + coverage/environment scanning, `--changed [since]` git filter,
  isolate/environment env wiring, and `--passWithNoTests` now run in Rust — `cli.js` is a thin
  binary-resolving shim. The binary launches a run with no Node-side logic; npm stays a
  distribution wrapper. *Why:* removes the Node process from launching, the first step to a
  self-contained binary. All compat suites green; binary self-discovers standalone.
- **P2a: native oxc ESM→CJS emitter (`src/esm_cjs.rs`), on by default.** Replaces the per-app-file
  `esbuild --format=cjs` transform with a hand-written oxc lowering (oxc 0.134 has no CommonJS
  module transform) that matches esbuild's output contract: live-binding member rewrites for
  named/default imports (scope-correct via semantic), `__export` getter block + `__toCommonJS`,
  `export *` via `__reExport`, `export default`, and dynamic `import()` →
  `Promise.resolve(__toESM(require(x)))`. Opt out with `TURBO_NATIVE_CJS=0`. *Why:* drops the
  esbuild subprocess for app files — a step toward removing the esbuild/npm dependency. esbuild
  is still used for node_modules bundling (P2b), coverage source maps, decorator-metadata, and as
  the automatic fallback for any unhandled form.
- **Conformity harness (`scripts/conformity.mjs`).** Runs a target project both ways (esbuild
  baseline vs native) and diffs per-file pass/fail — `parity` mode guarantees behavioral
  equivalence; `coverage` mode (native-strict) measures the native handling rate. *Why:* the
  safety mechanism gating the cutover; it already caught a real dynamic-`import()` bug, now fixed.
  Validated on the payroll-app `staging` worktree: **1057 files / 10471 tests, 100% native
  handling, full parity.**
- **P2b: native package bundler (`src/bundler.rs`), default ON.** Replaces esbuild for node_modules:
  bundles a package's relative graph with lazy `__commonJS` init wrappers (circular-safe), bare
  imports stay external (shared via require cache), assets stubbed. Opt out with `TURBO_NATIVE_DEPS=0`.
  Validated: payroll 1057 files / 10471 tests full parity with native app **+ deps** — so **normal
  test runs no longer use esbuild at all** (esbuild remains only for coverage, decorator-metadata,
  and as the fallback). Key fix: `__reExport` passes `module.exports` as its 3rd arg so names
  re-exported after `__toCommonJS` (e.g. `@testing-library/react`'s `render`) aren't lost.
- **P2c (partial): single-pass emit + native coverage source maps (gated).** `emit` now does TS-strip
  + ESM→CJS on one AST and, under coverage, appends a correct inline oxc source map. Coverage still
  runs on esbuild for now: oxc's codegen map is less dense than esbuild's, so `coverage.rs`
  under-attributes inner functions/lines. Fully removing esbuild additionally needs native
  decorator-metadata + dropping the fallback.

## [0.2.16] — vitest CLI/API compatibility sweep + turbo-dom 0.2.5

### Changed
- **Bump `@miaskiewicz/turbo-dom` `^0.2.2` → `^0.2.5`.** Pulls in the latest published DOM runtime.

### Added
- **vitest CLI compatibility — P0 batch.** Closes the highest-value gaps for being a drop-in
  `vitest run` in CI (see `vitest.compat.md` for the full audit + tracker):
  - `-t, --testNamePattern <re>` — run only tests whose full `describe > it` name matches the regex
    (unanchored, case-sensitive, matching vitest). Plumbed `cli.js` → `TURBO_TEST_NAME_PATTERN` env
    → `__TT_NAME_PATTERN` global → `runtime.js` `runSuite` filter.
  - Leading `run`/`watch`/`dev` subcommand token is accepted and stripped (turbo-test is always a
    single run), so the canonical `vitest run …` invocation works unchanged.
  - `--passWithNoTests` — exit 0 (not 1) when discovery finds no test files.
  - Unknown `-`/`--` flags are now warned-and-ignored instead of being treated as test-file paths
    (which previously reached the runner as a hard load-error and flipped the exit code).
- **vitest CLI compatibility — P1/P2 batches** (developed in parallel worktrees, merged together):
  - **Execution control:** `--testTimeout <ms>` + per-test `{ timeout }` now ENFORCED (internal
    one-shot timer raced against the body, invisible to `vi.runAllTimers`/`getTimerCount`; a hung
    `await new Promise(()=>{})` test now fails cleanly instead of hanging the worker); `--retry <n>`
    global default; `--bail <n>` (shared cross-worker failure counter, file-granular); `--maxWorkers`
    alias of `--jobs`, `--minWorkers` accepted (no-op); `--silent` (test `console.*` no-op);
    `--allowOnly`/`--no-allowOnly` (per-file `.only` gate that flips the exit code).
  - **Reporters/output:** `--reporter junit` (per-testcase XML), `tap` (TAP v13), `verbose`, `dot`,
    `default`; `--outputFile <path>` for json/junit/tap. Unknown reporter values fall back to text
    (never error). Required retaining a per-test result list (`TestReport.tests`) through
    `runtime.js` → `runner.rs` → `turbo_test.rs`.
  - **Config/discovery/environment:** `-c/--config <path>` (force exact config), `--root`/`--dir`
    (discovery-root override), `--environment <node|jsdom|happy-dom>` + `// @vitest-environment`
    pragma (`node` skips turbo-dom DOM-global install via `TURBO_ENV`), `--isolate`/`--no-isolate`
    (→ `TURBO_NO_REUSE`/`TURBO_REUSE_ISOLATE`), `--changed [since]` (git changed-file filter),
    `--globals`/`--no-globals` (accepted; `--no-globals` a documented no-op).
  - **Test/`expect` API:** file snapshots (`toMatchSnapshot`, `toThrowErrorMatchingSnapshot`) +
    `-u/--update`; `toMatchInlineSnapshot` compare-only (no source auto-write); `expect.assertions(n)`/
    `hasAssertions()` enforcement; `it.fails`; `describe.todo/.skipIf/.runIf/.concurrent`; `it.extend`
    fixtures (best-effort).
  - Documented compatibility gaps for every partial item are tracked in `vitest.compat.md`.

### Tests
- Added `test/cli-compat.test.mjs` (the `test/` dir `npm test`/`node --test` already expected but
  which did not exist) + `test/compat-{runflags,reporters,config-env,api}.test.mjs` — **47 cases**
  total locking the behaviors above, plus `fixtures/compat/`.

### Docs
- Added `vitest.compat.md` — a living CLI/command + `vi`/`expect` API compatibility matrix and
  prioritized backlog, updated as each batch landed.

## [0.2.15] — bump turbo-dom to 0.2.2

### Changed
- **Bump `@miaskiewicz/turbo-dom` `^0.2.0` → `^0.2.2`.** Pulls in the latest published DOM
  runtime so consumers installing fresh resolve the current line rather than the older 0.2.0
  floor. No turbo-test source changes; patch release to ship the updated dependency range.

## [0.2.14] — E12: memoize module resolution (tsconfig + specifier resolve)

### Performance
- **Memoize `nearest_tsconfig` and `resolve_spec_as` per worker** (thread-local). `native_require`
  was the second-hottest path in the profile and re-walked the directory tree (`is_file` syscalls)
  for the nearest `tsconfig.json` on *every* resolve, and re-ran the full oxc resolve + canonicalize
  for the same `(specifier, importer-dir)` across every file that imports it. Both are deterministic
  for a run, so caching them turns repeats into a hashmap hit and cuts the FS-syscall + resolution
  cost. **Default ON** (disable with `TURBO_NO_E12`).
  - Paired A/B vs the validated noise floor, identical pass/fail on both suites (accuracy-diff:
    ui 431/431 and payroll 1072/1072 files byte-identical pass/fail):
    micro ui −13.8% / payroll −14.1% (20/20); **full-suite ui −7.4% (7/8) / payroll −13.9% (4/4)**.

### Added
- `scripts/perf/harness.sh --alt`: alternate A/B run order per pair. Cancels a deterministic
  *within-pair* thermal bias (at `--jobs 1` the second run of each pair was ~46% slower as the
  pinned P-core throttled — a control-vs-control run showed +46%/0-of-12, pure artifact). The
  earlier "validated ±0.4% noise floor" was a **jobs=8** measurement; jobs=8 has only random
  variance (cancels over pairs), jobs=1 has this deterministic bias (does not). Use jobs=8 or
  `--alt` for per-file micro A/B.

### Investigated (not shipped)
- **E1 V8 `--max-semi-space-size=64`**: re-benchmarked on a quiet machine — neutral-to-slightly
  slower (+2.7% at jobs=8). The earlier "−10% promising" was jobs=1 thermal-bias inflation. Killed.
- **E4 worker count** (`ncpu→7/6`): full-suite +15.9% / +59.8% slower. Killed (reconfirmed).
- **E10 V8 platform helper-pool size**: capping `new_default_platform(N,…)` to 2/4 was +11.3% /
  +5.3% slower full-suite — fewer GC helpers idles concurrent-GC parallelism. Killed. (Both
  concurrency levers — worker count and helper pool — are closed: ncpu workers + ncpu helpers is
  the full-cold-suite optimum.)
- **E6 transform-existence memo**: neutral (+0.0%). **E11 drain-loop fn-lookup hoist**: +3.9%
  (the per-iteration `v8::String::new` churn is negligible vs total). Neither shipped.

### Added (earlier perf-spike, behavior identical to 0.2.12)
- `scripts/perf/` harness (`harness.sh` micro/ab/full/profile, `accuracy-diff.sh`) + README.
- Sweep env gates: `TURBO_V8_FLAGS`, `TURBO_JOBS`, `TURBO_SNAP_KEEP`, `TURBO_NO_CODE_CACHE`.
- `docs/`: `perf-spike.md`, `reuse-spike.md`, `TODO-cache-poisoning.md`.

### Investigated earlier (not shipped)
- Isolate-reuse as default: **rejected** — faster on some suites (ui 7006/0) but breaks payroll
  (per-file `vi.mock` of node_modules is incompatible with caching node_modules across files).
  Stays opt-in (`TURBO_REUSE_ISOLATE` / vitest `isolate: false`).
- Snapshot bytecode bake (`Keep`): mixed (ui −3.6%, payroll neutral/slower); kept opt-in.
- In-memory dep-bundle memo: rejected (slower — clone beats OS-page-cached read).

## [0.2.13] — `toEqual` undefined-own-property stripping
### Fixed
- `toEqual` / `toHaveBeenCalledWith` / `toStrictEqual`'s negation and all the matchers backed by
  the deep-equality path now follow jest/vitest semantics: an own property whose value is
  `undefined` is treated as **absent**, so `{ a: 1, b: undefined }` deep-equals `{ a: 1 }`.
  Previously `deepEqual` compared raw `Object.keys().length`, so any received object carrying an
  explicit-`undefined` key failed against an expectation that omitted it — and because the
  `undefined` value isn't rendered, the printed `expected` and `received` were byte-identical,
  making the failure look impossible. (`BUG-equality-state-leak-across-shard-files.md`.)
- The bug was reported as a shard/worker **state leak** (intermittent, order-dependent). It is not:
  the gap is deterministic and fires whenever the received object has an explicit-`undefined` own
  property. It only *looked* shard-dependent because whether the asserted object carried such a key
  depended on which code path the consumer's earlier files exercised.
- `toStrictEqual` keeps `undefined`-valued keys significant (now via an explicit `strict` flag),
  matching jest/vitest. Array elements are never stripped in either mode — `[1, undefined]` still
  differs from `[1]` because the length differs.
### Tests
- `fixtures/tests/equality-undefined-strip.test.mjs` — regression covering the reported
  `toHaveBeenCalledWith({...})` shape, nested objects, `toStrictEqual`, and array non-stripping.

## [0.2.12] — V8 bytecode code-cache
### Added
- Persistent V8 code (bytecode) cache for compiled CJS module wrappers, keyed by the exact wrapped
  source, consumed across isolates/workers/runs. A fresh isolate per test file otherwise re-parses
  + re-compiles every required module (incl. node_modules barrels) from scratch. On by default;
  disable with `TURBO_NO_CODE_CACHE`; skipped under coverage. Safe: V8 rejects a stale/mismatched
  cache and recompiles.
### Performance
- ~1.5–1.8% faster warm (paired A/B); identical pass/fail (payroll 10580/1, ui 7006/0).

## [0.2.11] — experimental jest drop-in
### Added
- Experimental jest compatibility: `jest`-global shim, jest config reading, `emitDecoratorMetadata`
  support, CJS-first resolution path (sequelize/tslib/lexical get their `require`-condition build).

## [0.2.10] — mock + dual-React fixes
### Fixed
- Dual-React instance in setup-file bundles (mock factories now use the test's React).
- `vi.mock` factories that close over outer `let`s (shared via a routed global).
- ESM deps resolved under the wrong node export condition.

## [0.2.9] — async self-importing mock factories
### Fixed
- An async `vi.mock` factory that `await import('<self>')` now rebinds named imports to the spies.

## [0.2.8] — statements coverage accuracy
### Fixed
- Statements coverage previously counted declarations; now Istanbul-accurate (executable
  statements only, correlated to V8 covered ranges).

## [0.2.7] — coverage globs + fail-loud
### Fixed
- Brace globs (`{ts,tsx}`) in `coverage.include` were torn into malformed globs; the multi-glob
  separator is now a top-level comma only.
### Changed
- Under `--coverage`, 0 instrumented files is a hard FAIL (non-zero exit) instead of a vacuous 0/0.

## [0.2.6] — statements coverage
### Added
- `statements` coverage metric, derived via oxc (shared parse with the branch pass) correlated to
  V8 covered ranges. Appears in json-summary / text / html and is gateable (omitted from lcov).

## [0.2.5] — platform reporting
### Fixed
- Report the real host platform/arch so turbo-dom loads the matching prebuilt `.node` (previously
  reported darwin/arm64 everywhere → wrong `.node` on other hosts → "document is not defined").

## [0.2.4] — linux-x64 startup
### Fixed
- linux-x64 startup segfault (`e_entry=0` from a misparsed linker flag).

## [0.2.3] — coverage gating + reporters
### Added
- Coverage thresholds / gating; `json-summary` and `html` reporters.
### Fixed
- A branch-coverage correlation bug.

## [0.2.2] — function + branch coverage
### Added
- Function and branch coverage (oxc AST decision points correlated with V8 block counts).
### Fixed
- Config parser matched `setupFiles` / `isolate` inside comments instead of the real keys.

## [0.2.1] — native V8 coverage
### Added
- Native V8 line + function coverage via the Inspector Profiler; honors vitest
  `coverage.include` / `exclude`.

## [0.2.0] — initial public release
First published `@miaskiewicz/turbo-test`: a native Rust + V8 vitest-compatible test runner.
### Added
- Native multi-worker runner: one V8 isolate per worker booted from a shared framework snapshot,
  work-stealing scheduler, duration-aware slowest-first ordering.
- vitest-compatible surface: `describe`/`it`/`expect`, `vi.mock`/`vi.fn`/`vi.spyOn`, hooks, timers,
  module-runner CJS loading with live bindings + shared React, vitest config honoring
  (include/exclude, setupFiles, environment).
- Isolate-reuse mode (vitest `isolate: false` / `TURBO_REUSE_ISOLATE`) with a fresh-isolate retry
  net; node builtin shims, virtual clock, e2e dep-stubbing.
- npm distribution: `cli.js` launcher over prebuilt per-platform binaries, CI matrix build +
  tag-triggered publish (macOS arm64, Linux x64 gnu/musl, Windows x64).
- Validated against real suites: payroll-app 100%, ui-design-components 6189/0; ~5.6–9× vs
  vitest+jsdom in apples-to-apples benchmarks.
