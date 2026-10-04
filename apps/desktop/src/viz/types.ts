// What every visualizer scene gets each frame, and how a scene is made.
import type * as THREE from 'three';
import type { Features } from '../audiofeatures';

export interface Frame {
  dt: number;
  /** Seconds since the scene started. */
  time: number;
  f: Features;
  /** New hits this frame (each starts its own wave or ring). */
  hit: { kick: boolean; snare: boolean; hat: boolean };
  /** The person prefers less motion. */
  calm: boolean;
  width: number;
  height: number;
}

export interface Scene {
  frame(fr: Frame): void;
  resize?(w: number, h: number): void;
  dispose(): void;
}

export type MakeScene = (renderer: THREE.WebGLRenderer) => Scene;

/** Colors: a rainbow that keeps cycling (from a random point each time the app opens), so no color owns a
 *  song. What a sound does shows in how its color is drawn, not which color it is:
 *    low  (kick, bass)    deep and rich, a touch warmer, with a wide soft halo
 *    high (snare, hats)   pale and bright, a touch cooler, with a tight crisp core
 *    voice                medium and smooth, the hue leaning with the note
 *  Shaders using these need `uTime` declared before this block. */
const HUE0 = Math.random().toFixed(3);
export const GLSL_COLORS = `
vec3 rainbow(float x){return .55+.45*cos(6.2831*(x+vec3(0.,.33,.67)));}
float hueAt(float x){return uTime*.021+${HUE0}+x;}
vec3 satur(vec3 c,float s){float l=dot(c,vec3(.299,.587,.114));return mix(vec3(l),c,s);}
vec3 temper(vec3 c,float k){return c*vec3(1.+.22*k,1.,1.-.22*k);}
vec3 lowColor(float x){return temper(satur(rainbow(hueAt(x)),1.2),.35)*1.2;}
vec3 highColor(float x){return temper(satur(rainbow(hueAt(x)),.4),-.3)*.9+.18;}
vec3 voiceColor(float x,float pitch){return satur(rainbow(hueAt(x+pitch*.25)),.8);}
`;

/** Recent hits, newest first, as ages in seconds: each one keeps travelling while new ones start. */
export class Ripples {
  ages: number[];
  amps: number[];
  constructor(n = 8) {
    this.ages = Array(n).fill(99);
    this.amps = Array(n).fill(0);
  }
  add(amp: number) {
    this.ages.unshift(0);
    this.ages.pop();
    this.amps.unshift(amp);
    this.amps.pop();
  }
  step(dt: number) {
    for (let i = 0; i < this.ages.length; i++) this.ages[i] += dt;
  }
}

/** A full-screen shader scene (one quad, everything drawn in the fragment shader). */
export function shaderScene(THREE_: typeof THREE, renderer: THREE.WebGLRenderer, fragment: string, uniforms: Record<string, THREE.IUniform>) {
  const scene = new THREE_.Scene();
  const cam = new THREE_.OrthographicCamera(-1, 1, 1, -1, 0, 1);
  const mat = new THREE_.ShaderMaterial({
    uniforms: { uRes: { value: new THREE_.Vector2(1, 1) }, uTime: { value: 0 }, ...uniforms },
    vertexShader: 'varying vec2 vUv;void main(){vUv=uv;gl_Position=vec4(position.xy,0.,1.);}',
    fragmentShader: fragment,
    depthWrite: false,
  });
  const quad = new THREE_.Mesh(new THREE_.PlaneGeometry(2, 2), mat);
  scene.add(quad);
  return {
    mat,
    render(w: number, h: number, time: number) {
      (mat.uniforms.uRes.value as THREE.Vector2).set(w, h);
      mat.uniforms.uTime.value = time;
      renderer.render(scene, cam);
    },
    dispose() {
      quad.geometry.dispose();
      mat.dispose();
    },
  };
}
