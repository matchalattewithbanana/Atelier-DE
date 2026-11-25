//_DEFINES_
#ifdef EXTERNAL
#extension GL_OES_EGL_image_external : require
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif
varying vec2 v_coords;
uniform float alpha;
uniform vec4 mask_rect;
uniform vec2 mask_scale;
uniform vec2 mask_y;
uniform float mask_radius;
#ifdef DEBUG_FLAGS
uniform float tint;
#endif
void main() {
    vec2 physical = vec2(gl_FragCoord.x, mix(gl_FragCoord.y, mask_y.y - gl_FragCoord.y, mask_y.x));
    vec2 point = physical / mask_scale;
    vec2 half_size = mask_rect.zw * 0.5;
    vec2 center = mask_rect.xy + half_size;
    float r = min(mask_radius, min(half_size.x, half_size.y));
    vec2 q = abs(point - center) - half_size + vec2(r);
    float distance_to_edge = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
    float edge = 0.5 / min(mask_scale.x, mask_scale.y);
    float coverage = 1.0 - smoothstep(-edge, edge, distance_to_edge);
    vec4 color = texture2D(tex, v_coords);
#ifdef NO_ALPHA
    color.a = 1.0;
#endif
#ifdef DEBUG_FLAGS
    color.rgb = mix(color.rgb, vec3(0.0, 0.2, 0.0) * color.a, tint * 0.2);
#endif
    gl_FragColor = color * (alpha * coverage);
}
