# P9 and R1 - the user's instruction (2026-10-10 ~00:05 UTC), verbatim

Add in the handoff scripr to do a first release with market and fix splitted, add instructions to replace current isin registry into our own market/instrument.rs
With our Instrument: graph Element implementations containing all mapping for isin, cfi, lei,ric with learnings from market recording / fix parsing
Thus filling the value of market instrumentuuid and dumped in medaillon

Make the intrument then be able to create custom isin codes, fill country currency

Best usecase is autocreating for forex pairs which dont have existing isin and create instruments

Add then optional instrument underlying pointing to another indtrument uuid definiing an underlying and legs uuids list dedining legs

Add then charzcteristrics to handle generic finance products like future options strikepx or other charzcteristics and include it syntheticqlly in cross code then correct grzph defined hashing in uuids

Thus instrument cross uuid should rely on cficode + isincode

And fill the market intrumentuuid with this cross uuid
