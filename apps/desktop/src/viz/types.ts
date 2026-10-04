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

/** Instrument colors, the same in every scene: low (kick, bass) ember, high (snare, hats) frost, voice gold → rose. */
export const GLSL_COLORS = `
const vec3 EMBER = vec3(1.,.42,.16);
const vec3 FROST = vec3(.55,.9,1.);
vec3 voiceColor(float pitch){return mix(vec3(1.,.78,.3),vec3(1.,.38,.62),clamp(pitch,0.,1.));}
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
