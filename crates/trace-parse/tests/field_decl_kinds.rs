use trace_parse::parse_c_source;

#[test]
fn dump_net_device_impl_op_field_decl() {
    let src = r#"
struct NetDeviceImplOp {
    int32_t (*setIpAddr)(struct NetDeviceImpl *netDevice, const IpV4Addr *ipAddr);
    int32_t (*init)(struct NetDeviceImpl *netDevice);
};
"#;
    let parsed = parse_c_source(src).unwrap();
    fn walk(node: tree_sitter::Node, depth: usize) {
        if depth <= 8 {
            println!("{}{}", "  ".repeat(depth), node.kind());
        }
        let mut c = node.walk();
        for ch in node.children(&mut c) {
            walk(ch, depth + 1);
        }
    }
    walk(parsed.tree.root_node(), 0);
    let body = parsed
        .tree
        .root_node()
        .descendant_for_byte_range(0, parsed.source.len())
        .unwrap();
    fn find_field(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
        if node.kind() == "field_declaration" {
            return Some(node);
        }
        let mut c = node.walk();
        for ch in node.children(&mut c) {
            if let Some(n) = find_field(ch) {
                return Some(n);
            }
        }
        None
    }
    let fd = find_field(body).expect("field_declaration");
    let struct_node = parsed.tree.root_node().named_child(0).unwrap();
    println!(
        "body field: {:?}",
        struct_node.child_by_field_name("body").map(|n| n.kind())
    );
    println!(
        "field_declaration_list: {:?}",
        struct_node
            .child_by_field_name("field_declaration_list")
            .map(|n| n.kind())
    );
    println!(
        "declarator field: {:?}",
        fd.child_by_field_name("declarator").map(|n| n.kind())
    );
    println!(
        "type field: {:?}",
        fd.child_by_field_name("type").map(|n| n.kind())
    );
}
