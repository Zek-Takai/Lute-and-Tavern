// Mandala: a slowly turning kaleidoscope. Each kick sends an ember ring outward from the center and each
// snare a thin frost ring, so rings ride out one after another like ripples; the voice makes the gold → rose
// petals breathe; hi-hats light the frost lace at the edge; the bass warms the center.
import * as THREE from 'three';
import { Ease, GLSL_NOISE } from './noise';
import { GLSL_COLORS, Ripples, shaderScene, type Frame, type MakeScene } from './types';

const FRAG = `precision highp float;varying vec2 vUv;uniform vec2 uRes;uniform float uTime,uLow,uVoice,uPitch,uHat,uInt,uSpin;
uniform float uKickAge[8],uKickAmp[8],uSnareAge[8],uSnareAmp[8];${GLSL_NOISE}${GLSL_COLORS}
void main(){
  vec2 p=(vUv-.5)*vec2(uRes.x/uRes.y,1.)*2.;
  float r=length(p),a=atan(p.y,p.x)+uSpin;
  float n=8.,seg=6.2831/n;
  float aa=abs(mod(a,seg)-seg*.5);
  vec3 col=vec3(.02,.02,.06)+vec3(.04,.02,.08)*(1.-r);
  float petal=.32+.1*sin(aa*6.+uTime*.6)+uVoice*.18;
  float pe=smoothstep(petal,petal-.03,r)*smoothstep(.0,.12,aa*r*4.)*(.35+.65*fbm(vec2(aa*4.,r*5.-uTime*.3)));
  col+=voiceColor(aa*.4+r*.3,uPitch)*pe*(.15+uVoice*.8);
  float inner=.55+.08*cos(aa*12.-uTime*.4);
  col+=rainbow(hueAt(.25+a*.16))*smoothstep(.012,0.,abs(r-inner))*(.2+uInt*.5);
  for(int i=0;i<8;i++){
    float fi=float(i);
    float k=r-uKickAge[i]*.45;col+=lowColor(fi*.13)*(exp(-k*k*900.)+exp(-k*k*90.)*.3)*uKickAmp[i]*exp(-uKickAge[i]*.9);
    float s=r-uSnareAge[i]*.6;col+=highColor(.5+fi*.11)*(exp(-s*s*5000.)*1.3+exp(-s*s*800.)*.12)*uSnareAmp[i]*exp(-uSnareAge[i]*1.3);
  }
  float lace=smoothstep(.04,0.,abs(r-.9))*(.5+.5*sin(aa*48.+uTime*1.3));
  col+=highColor(a*.16)*lace*(.08+uHat*.9);
  col+=lowColor(.7)*(exp(-r*r*14.)+exp(-r*r*3.)*.25)*uLow*.55;
  col*=smoothstep(1.35,.9,r);
  gl_FragColor=vec4(col,1.);
}`;

export const makeMandala: MakeScene = (renderer) => {
  const kicks = new Ripples(8), snares = new Ripples(8);
  const U = {
    uLow: { value: 0 }, uVoice: { value: 0 }, uPitch: { value: 0.5 }, uHat: { value: 0 }, uInt: { value: 0.3 }, uSpin: { value: 0 },
    uKickAge: { value: kicks.ages }, uKickAmp: { value: kicks.amps }, uSnareAge: { value: snares.ages }, uSnareAmp: { value: snares.amps },
  };
  const s = shaderScene(THREE, renderer, FRAG, U);
  const low = new Ease(3, 1), voice = new Ease(3, 1.2), hat = new Ease(8, 2), inten = new Ease(0.6, 0.6), pitch = new Ease(2, 2);
  let t = 0, spin = 0;
  return {
    frame({ dt, f, hit, calm, width, height }: Frame) {
      const k = calm ? 0.3 : 1;
      t += dt * k;
      if (hit.kick) kicks.add((0.6 + f.bass * 0.4) * (calm ? 0.4 : 1));
      if (hit.snare) snares.add(calm ? 0.3 : 0.8);
      kicks.step(dt);
      snares.step(dt);
      U.uInt.value = inten.to(f.intensity, dt);
      spin += dt * k * (0.04 + (U.uInt.value as number) * 0.08);
      U.uSpin.value = spin;
      U.uLow.value = low.to(f.bass, dt);
      U.uVoice.value = voice.to(f.vocal, dt);
      U.uPitch.value = pitch.to(f.pitch, dt);
      U.uHat.value = hat.to(f.hat, dt);
      s.render(width, height, t);
    },
    dispose: () => s.dispose(),
  };
};
