// Premultiplied smooth outer border, with the client interior excluded.
precision mediump float;
varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;
uniform vec4 outline;
uniform float corner;
uniform float width;
uniform vec4 border_color;
#ifdef DEBUG_FLAGS
uniform float tint;
#endif
void main() {
    vec2 p = v_coords * size;
    vec2 half_size = (outline.zw - outline.xy) * 0.5;
    float r = min(corner, min(half_size.x, half_size.y));
    vec2 q = abs(p - (outline.xy + outline.zw) * 0.5) - half_size + vec2(r);
    float d = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
    // Keep the hard inner boundary consistent with the existing client
    // clip: never shade through translucent content inside that shape.
    if (d < 0.0) discard;
    float coverage = 1.0 - smoothstep(width - 0.5, width + 0.5, d);
    float a = coverage * border_color.a * alpha;
    vec3 color = border_color.rgb;
#ifdef DEBUG_FLAGS
    color = mix(color, vec3(0.0, 0.2, 0.0), tint * 0.2);
#endif
    gl_FragColor = vec4(color * a, a);
}
