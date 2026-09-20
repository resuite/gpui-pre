const SHADERS: &str = include_str!("../src/shaders.hlsl");

fn fields(struct_name: &str) -> Vec<String> {
    let declaration = format!("struct {struct_name} {{");
    let (_, body) = SHADERS
        .split_once(&declaration)
        .unwrap_or_else(|| panic!("missing {struct_name}"));
    let (body, _) = body.split_once("};").expect("unterminated HLSL struct");
    body.lines()
        .map(|line| line.split("//").next().expect("line has a first segment"))
        .flat_map(|line| line.split(';'))
        .map(|field| field.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|field| !field.is_empty())
        .collect()
}

#[test]
fn transformed_shader_interfaces_match_including_clip_distance() {
    // D3D11 links registers, not just semantic names. Omitting clip distance
    // between consumed fields can shift their compiled register assignments.
    for primitive in [
        "Quad",
        "Shadow",
        "Underline",
        "MonochromeSprite",
        "PolychromeSprite",
    ] {
        let vertex = fields(&format!("{primitive}TransformedVertexOutput"));
        let fragment = fields(&format!("{primitive}TransformedFragmentInput"));
        assert!(
            vertex.iter().any(|field| field.contains("SV_ClipDistance")),
            "{primitive} must retain hardware clipping"
        );
        assert_eq!(vertex, fragment, "{primitive} shader interfaces differ");
    }
}
