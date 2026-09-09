use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, Throughput};
use yggdryl::holder::fs::{File, FileSystem, MemoryFileSystem};
use yggdryl::{DataType, FixBranch, FixCategory, FixId, FixRegistry, IOBase};

/// How many vocabulary tags and bound constraints one measured file holds.
///
/// A production CBlock is megabytes over hundreds of thousands of elements;
/// this is the same shape at a size a benchmark can run repeatedly.
const TAGS: usize = crate::bench_profile::corpus(2_000, 200);

/// A CBlock of `TAGS` tags, including a counter and its nested group member.
fn document() -> String {
    let mut body = String::with_capacity(TAGS * 220);
    body.push_str(
        "<?xml version=\"1.0\" encoding=\"US-ASCII\"?>\n\
         <cplugin-configuration type=\"BuySideFIXCPluginCBlock\" version=\"1.2\" \
         fix-version=\"4.4\">\n\
         \t<history><version date=\"2006-09-06\" owner=\"ullink\">First version</version></history>\n\
         \t<message-types>\n",
    );
    for index in 0..TAGS {
        body.push_str(&format!(
            "\t\t<message-type value=\"{index}\" description=\"d\" rejection=\"r\" supported=\"false\" />\n"
        ));
    }
    body.push_str("\t</message-types>\n\t<vocabulary>\n");
    for index in 0..TAGS {
        let tag = 5_000 + index;
        let (name, dtype) = if index == 0 {
            ("NoVendorEntries".to_owned(), "integer")
        } else {
            (format!("Vendor{index:05}"), "string")
        };
        body.push_str(&format!(
            "\t\t<vocabulary-tag name=\"{tag}\" alt=\"{name}\" type=\"{dtype}\" read-only=\"false\">\n\
             \t\t\t<description>A field carrying a trailing &lt;SOH&gt; and a &quot;quoted&quot; word.</description>\n\
             \t\t</vocabulary-tag>\n"
        ));
    }
    body.push_str(
        "\t</vocabulary>\n\t<grammar-binding type=\"D\">\n\t\t<grammar checkordering=\"false\">\n",
    );
    body.push_str(
        "\t\t\t<grammar rg-name=\"VendorEntries\" checkordering=\"false\">\n\
         \t\t\t\t<tag-constraint name=\"5000\" part=\"body\" required=\"false\" />\n\
         \t\t\t\t<tag-constraint name=\"5001\" part=\"body\" required=\"false\" />\n\
         \t\t\t</grammar>\n",
    );
    for index in 2..TAGS {
        let tag = 5_000 + index;
        body.push_str(&format!(
            "\t\t\t<tag-constraint name=\"{tag}\" activated=\"true\" read-only=\"false\" part=\"body\" required=\"false\">\n\
             \t\t\t\t<string-validity regexp=\".*\" domain=\"all-values\" />\n\
             \t\t\t</tag-constraint>\n"
        ));
    }
    body.push_str("\t\t</grammar>\n\t</grammar-binding>\n\t<normalization-binding>\n\t\t<normalization type=\"inbound\">\n");
    for index in 0..TAGS {
        let tag = 5_000 + index;
        let name = if index == 0 {
            "NOVENDORENTRIES".to_owned()
        } else {
            format!("VENDOR{index:05}")
        };
        // The common case by far: a spelling the tag already answers to,
        // which the pass has to resolve and drop rather than store.
        body.push_str(&format!(
            "\t\t\t<tag-normalization tag-name=\"{name}\" part=\"body\">\n\
             \t\t\t\t<mapping-expression>\n\
             \t\t\t\t\t<expression value=\"${tag}\" />\n\
             \t\t\t\t</mapping-expression>\n\
             \t\t\t</tag-normalization>\n"
        ));
        // A lookup names nothing, and is what a real binding writes beside it.
        body.push_str(&format!(
            "\t\t\t<tag-normalization tag-name=\"{name}CODE\" part=\"body\">\n\
             \t\t\t\t<mapping-condition>\n\
             \t\t\t\t\t<expression value=\"${tag} = &quot;4&quot;\" />\n\
             \t\t\t\t</mapping-condition>\n\
             \t\t\t\t<mapping-expression>\n\
             \t\t\t\t\t<expression value=\"lookup(&quot;Set&quot;, ${tag})\" />\n\
             \t\t\t\t</mapping-expression>\n\
             \t\t\t</tag-normalization>\n"
        ));
        // And one in ten is a spelling the tag does not answer to, so the
        // measured pass writes as well as reads.
        if index % 10 == 0 {
            body.push_str(&format!(
                "\t\t\t<tag-normalization tag-name=\"{name}_ALT\" part=\"body\">\n\
                 \t\t\t\t<mapping-expression>\n\
                 \t\t\t\t\t<expression value=\"${tag}\" />\n\
                 \t\t\t\t</mapping-expression>\n\
                 \t\t\t</tag-normalization>\n"
            ));
        }
    }
    body.push_str(
        "\t\t</normalization>\n\t</normalization-binding>\n\t<reject-binding />\n</cplugin-configuration>\n",
    );
    body
}

/// One document behind a handle.
fn handle(body: &str) -> impl IOBase {
    let filesystem: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    filesystem.create_dir("cblock", true).expect("a container");
    let mut file = File::from_path(filesystem, "cblock/bench.cfb", None).expect("a path");
    file.write_all_bytes(body.as_bytes()).expect("the document");
    file
}

pub fn benchmarks(criterion: &mut Criterion) {
    let body = document();
    let handle = handle(&body);
    let branch = FixBranch::from_str("venue").expect("a valid branch");
    let (registry, _) =
        FixRegistry::from_cfb_file(&handle, Some(&branch)).expect("a readable CBlock");
    let counter = FixId::from_parts(&branch, 5_000).unwrap();
    assert_eq!(registry.field(counter).unwrap().dtype(), &DataType::Int32);
    assert!(
        registry
            .get_definition(FixCategory::Groups, "VendorEntries", Some(&branch))
            .is_some()
    );
    assert!(
        registry
            .msgtype("D", Some(&branch))
            .unwrap()
            .get_group_by_counter(counter)
            .is_some()
    );
    // The normalization binding spells every tag, and only the spelling the
    // tag does not already answer to is stored beside its name.
    assert_eq!(
        registry
            .field(counter)
            .unwrap()
            .as_fix()
            .aliases()
            .collect::<Vec<_>>(),
        ["NOVENDORENTRIES_ALT"]
    );

    let mut group = criterion.benchmark_group("fix/cblock");
    group.throughput(Throughput::Bytes(body.len() as u64));
    // The whole parse: skip unrelated children, build the vocabulary, bind
    // the message and its group from the resolved fields, then resolve every
    // name the normalization binding spells against the finished vocabulary.
    group.bench_function("parse", |bencher| {
        bencher.iter(|| {
            FixRegistry::from_cfb_file(black_box(&handle), Some(black_box(&branch)))
                .expect("a readable CBlock")
        });
    });
    group.finish();
}
