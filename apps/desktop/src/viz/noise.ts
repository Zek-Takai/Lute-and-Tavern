// Small shader helpers shared by the scenes.
export const GLSL_NOISE = `
float hash(vec2 p){return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453);}
float noise(vec2 p){vec2 i=floor(p),f=fract(p);f=f*f*(3.-2.*f);
  return mix(mix(hash(i),hash(i+vec2(1.,0.)),f.x),mix(hash(i+vec2(0.,1.)),hash(i+vec2(1.,1.)),f.x),f.y);}
float fbm(vec2 p){float v=0.,a=.5;for(int i=0;i<5;i++){v+=a*noise(p);p*=2.02;a*=.5;}return v;}
`;

/** A level that eases toward its target (up quickly, down slowly), so nothing jumps. */
export class Ease {
  v = 0;
  constructor(public up = 4, public down = 1.5) {}
  to(target: number, dt: number) {
    this.v += (target - this.v) * Math.min(1, dt * (target > this.v ? this.up : this.down));
    return this.v;
  }
}
