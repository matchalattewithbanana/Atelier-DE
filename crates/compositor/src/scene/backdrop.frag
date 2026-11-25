//_DEFINES_
#ifdef EXTERNAL
#extension GL_OES_EGL_image_external : require
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
precision mediump float;
varying vec2 v_coords;
uniform float alpha;
uniform vec2 step_uv;
#ifdef DEBUG_FLAGS
uniform float tint;
#endif
vec3 tap(vec2 offset) {
    return texture2D(tex, clamp(v_coords + offset * step_uv, vec2(0.0), vec2(1.0))).rgb;
}
void main() {
    // Fixed 13-tap approximate blur, normalized weights.
    vec3 color = tap(vec2(0.0)) * 4.0;
    color += (tap(vec2(0.5,0.0)) + tap(vec2(-0.5,0.0)) + tap(vec2(0.0,0.5)) + tap(vec2(0.0,-0.5))) * 2.0;
    color += tap(vec2(0.5,0.5)) + tap(vec2(-0.5,0.5)) + tap(vec2(0.5,-0.5)) + tap(vec2(-0.5,-0.5));
    color += tap(vec2(1.0,0.0)) + tap(vec2(-1.0,0.0)) + tap(vec2(0.0,1.0)) + tap(vec2(0.0,-1.0));
    color /= 20.0;
#ifdef DEBUG_FLAGS
    color = mix(color, vec3(0.0,0.2,0.0), tint * 0.2);
#endif
    gl_FragColor = vec4(color * alpha, alpha);
}
