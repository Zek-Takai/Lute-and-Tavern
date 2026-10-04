// Aurora: northern-lights curtains over a starry night. The bass glows along the horizon and each kick rolls a
// warm glow outward along it; the voice raises a gold → rose curtain that climbs with the pitch; hi-hats
// twinkle the stars; louder sections brighten the curtains (slowly).
import * as THREE from 'three';
import { Ease, GLSL_NOISE } from './noise';
import { GLSL_COLORS, Ripples, shaderScene, type Frame, type MakeScene } from './types';

const FRAG = `precision highp float;varying vec2 vUv;uniform vec2 uRes;uniform float uTime,uLow,uVoice,uPitch,uHat,uInt;
uniform float uKickAge[8],uKickAmp[8];${GLSL_NOISE}${GLSL_COLORS}
void main(){
  vec2 uv=vUv;float asp=uRes.x/uRes.y;vec2 p=vec2(uv.x*asp,uv.y);
  vec3 col=mix(vec3(.015,.02,.06),vec3(.04,.035,.11),uv.y);
  vec2 sp=p*170.;vec2 cell=floor(sp);float h=hash(cell);
  float star=step(.986,h)*smoothstep(.45,0.,length(fract(sp)-.5));
  float tw=.5+.5*sin(uTime*(1.+h*3.)+h*40.);
  col+=star*(.2+tw*.35)*vec3(.85,.9,1.)+star*step(.55,fract(h*13.+uTime*.35))*uHat*highColor(h)*1.5;
  for(int i=0;i<3;i++){float fi=float(i);
    float x=p.x*(.6+fi*.25)+uTime*(.025+fi*.012);
    float wave=fbm(vec2(x*1.8,uTime*.04+fi*3.))*.33;
    float base=.4+fi*.09+wave-(i==1?uVoice*.1*(.4+uPitch):0.);
    float d=uv.y-base;
    float curtain=smoothstep(0.,.025,d)*exp(-d*(4.5+fi*2.))*(.5+.5*fbm(vec2(x*7.,uTime*.15+fi)));
    vec3 c=rainbow(hueAt(fi*.27+x*.08));
    if(i==1)c=mix(c,voiceColor(x*.08,uPitch),clamp(uVoice*1.3,0.,1.));
    col+=c*curtain*(.22+uInt*.45+(i==1?uVoice*.55:0.));
  }
  float hy=uv.y-.16;float horizon=exp(-hy*hy*70.);
  float kw=0.;float dx=abs(uv.x-.5)*asp;
  vec3 lowc=vec3(0.);
  for(int i=0;i<8;i++){float k=dx-uKickAge[i]*.55;float a=uKickAmp[i]*exp(-uKickAge[i]*1.4);
    lowc+=lowColor(float(i)*.13)*(exp(-k*k*30.)+exp(-k*k*5.)*.35)*a;}
  col+=horizon*(lowc*.8+lowColor(.5)*uLow*.35);
  gl_FragColor=vec4(col,1.);
}`;

export const makeAurora: MakeScene = (renderer) => {
  const kicks = new Ripples(8);
  const U = {
    uLow: { value: 0 }, uVoice: { value: 0 }, uPitch: { value: 0.5 }, uHat: { value: 0 }, uInt: { value: 0.3 },
    uKickAge: { value: kicks.ages }, uKickAmp: { value: kicks.amps },
  };
  const s = shaderScene(THREE, renderer, FRAG, U);
  const low = new Ease(3, 1), voice = new Ease(3, 1), hat = new Ease(8, 2), inten = new Ease(0.6, 0.6), pitch = new Ease(2, 2);
  let t = 0;
  return {
    frame({ dt, f, hit, calm, width, height }: Frame) {
      t += dt * (calm ? 0.3 : 1);
      if (hit.kick && !calm) kicks.add(0.6 + f.bass * 0.4);
      kicks.step(dt);
      U.uLow.value = low.to(f.bass, dt);
      U.uVoice.value = voice.to(f.vocal, dt);
      U.uPitch.value = pitch.to(f.pitch, dt);
      U.uHat.value = hat.to(f.hat, dt);
      U.uInt.value = inten.to(f.intensity, dt);
      s.render(width, height, t);
    },
    dispose: () => s.dispose(),
  };
};
