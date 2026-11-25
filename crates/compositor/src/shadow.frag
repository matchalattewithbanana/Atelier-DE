// Smithay pixel shader: coordinates are normalized within each strip.
precision mediump float;
varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;
uniform vec4 outline;
uniform float radius;
#ifdef DEBUG_FLAGS
uniform float tint;
#endif
void main() {
    vec2 p = v_coords * size;
    vec2 outside = max(max(outline.xy - p, p - outline.zw), vec2(0.0));
    float distance_from_edge = length(outside);
    float fade = 1.0 - clamp(distance_from_edge / max(radius, 1.0), 0.0, 1.0);
    float a = alpha * fade * fade;
    vec3 color = vec3(0.0);
#ifdef DEBUG_FLAGS
    color = vec3(0.0, 0.2, 0.0) * tint;
#endif
    gl_FragColor = vec4(color * a, a);
}
