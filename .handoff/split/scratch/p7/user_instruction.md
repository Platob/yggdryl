# P7 - the user's instruction (2026-10-09 ~17:25 UTC), verbatim

And adapt fixmsg generated schemas to match fix registry names when same as our market like sendunix = sendingtime, transunix = transactime, strikepx = strikeprice etc...
Simplify the crossuuid to be simple xxh3 of crosscode uuid

Add the book.srcuuids all the unique sorted events constituting the book and set the book.haqhcode simply the transunix with the hash of srcuuids

Ensure to keep crate ordered fields from element to market operations and put at leaves the lift fields

Consider also the isincode, cficode as lifted  fields put at the end and keep indexed using securityids as only hold map and add instuuid just before for later instrument centralization

Rename cficode to cfi, miccode to mic and other codes

(Earlier, same session: "Focus on landing market and fix split validated by medaillon python test" - so P6+S4 land first; this slice follows inside the crates.)


# Refinement (2026-10-09 ~17:50 UTC), verbatim

on the with previous add also the previous uuid in it, but ensure srcuuids dont accumulate all uuids of all cycle events
