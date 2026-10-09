import sys, pathlib
S = pathlib.Path("/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad")
sys.path.insert(0, str(S))
import s4_move as s4
s4.CRATE_NAME["s3"] = "yggdryl_s3"
class T:
    def resolve(self, segs, routed=False):
        if segs and segs[0] == "s3":
            return "s3", segs[1:]
        if segs[:1] == ["internals"] and len(segs) > 1 and segs[1].startswith("s3_"):
            return "s3", ["internals", segs[1][3:], *segs[2:]]
        routes = {("http","retry","delay"): "delay", ("xml","scanner","Element"): "ScannedElement"}
        for cut in range(len(segs), 0, -1):
            n = routes.get(tuple(segs[:cut]))
            if n: return "core", ["implementer", n, *segs[cut:]]
        return "core", segs
r = s4.Rewriter(T())
out = s4.Context(None, False)
inside = s4.Context("s3", True)
for t in ["use yggdryl::s3::{self, Provider, S3Options};\nfn f() { s3::file(\"s3://x\"); }\n",
          "use yggdryl::s3;\n", "use yggdryl::s3::{S3Path, S3_BACKEND};\n",
          "/// ```\n/// use yggdryl::s3;\n/// let p = s3::file(\"s3://x\")?;\n/// ```\nfn g(){}\n",
          "use yggdryl::internals::s3_client::{Client, backoff};\n"]:
    print(r.rs_file(t, out, "x"))
    print("---")
for t in ["use crate::http::retry::{delay, RETRY_COST};\nuse crate::xml::scanner::{Element, XmlError, parse_root};\nuse crate::s3::{Credentials, Provider};\nuse crate::{Error, Result};\nfn h() { crate::s3::file(\"x\"); crate::aws::Session::new(); }\n"]:
    print(r.rs_file(t, inside, "y"))
