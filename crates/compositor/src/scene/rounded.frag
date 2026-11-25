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
uniform vec2 shape_size;
uniform float corner_radius;
#ifdef DEBUG_FLAGS
uniform float tint;
#endif
void main() {
    vec2 half_size = shape_size * 0.5;
    vec2 q = abs(v_coords * shape_size - half_size) - (half_size - vec2(corner_radius));
    float distance_to_edge = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - corner_radius;
    float coverage = clamp(0.5 - distance_to_edge, 0.0, 1.0);
    vec4 color = texture2D(tex, v_coords);
#ifdef DEBUG_FLAGS
    color.rgb = mix(color.rgb, vec3(0.0, 0.2, 0.0) * color.a, tint * 0.2);
#endif
    gl_FragColor = color * (coverage * alpha);
}
