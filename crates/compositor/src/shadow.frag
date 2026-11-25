// Smithay pixel shader: coordinates are normalized within each strip.
precision mediump float;
varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;
uniform vec4 outline;
uniform float radius;
uniform float corner;
#ifdef DEBUG_FLAGS
uniform float tint;
#endif
void main() {
    vec2 p = v_coords * size;
    vec2 half_size = (outline.zw - outline.xy) * 0.5;
    float r = min(corner, min(half_size.x, half_size.y));
    vec2 q = abs(p - (outline.xy + outline.zw) * 0.5) - half_size + vec2(r);
    float distance_from_edge = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
    if (distance_from_edge < 0.0) discard;
    float fade = 1.0 - clamp(distance_from_edge / max(radius, 1.0), 0.0, 1.0);
    float a = alpha * fade * fade;
    vec3 color = vec3(0.0);
#ifdef DEBUG_FLAGS
    color = vec3(0.0, 0.2, 0.0) * tint;
#endif
    gl_FragColor = vec4(color * a, a);
}
