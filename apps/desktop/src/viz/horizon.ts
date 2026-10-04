// Horizon: a retro grid running toward a setting sun, with mountains drawn from the last few seconds of the
// music. Each kick sends a band of ember light along the grid toward you (one after another); the sun takes
// the voice's gold → rose and swells while it sings; hi-hats sparkle the stars; louder sections speed the run.
import * as THREE from 'three';
import { Ease, GLSL_NOISE } from './noise';
import { GLSL_COLORS, Ripples, shaderScene, type Frame, type MakeScene } from './types';

const HIST = 64;
const FRAG = `precision highp float;varying vec2 vUv;uniform vec2 uRes;uniform float uTime,uTravel,uLow,uVoice,uPitch,uHat,uInt;
uniform float uKickAge[8],uKickAmp[8],uHist[${HIST}];${GLSL_NOISE}${GLSL_COLORS}
float hist(float x){float i=clamp(x,0.,1.)*${HIST - 1}.;int a=int(floor(i));int b=int(min(floor(i)+1.,${HIST - 1}.));
  float va=0.,vb=0.;for(int k=0;k<${HIST};k++){if(k==a)va=uHist[k];if(k==b)vb=uHist[k];}return mix(va,vb,fract(i));}
void main(){
  float asp=uRes.x/uRes.y;vec2 uv=(vUv-vec2(.5,.42))*vec2(asp,1.);
  vec3 col;
  if(uv.y>0.){
    col=mix(vec3(.12,.04,.18),vec3(.02,.02,.07),smoothstep(0.,.55,uv.y));
    vec2 sp=vUv*vec2(asp,1.)*160.;float h=hash(floor(sp));
    col+=step(.988,h)*smoothstep(.45,0.,length(fract(sp)-.5))*((.25)*vec3(.85,.9,1.)+uHat*1.3*step(.5,fract(h*7.+uTime*.4))*highColor(h));
    vec2 sc=uv-vec2(0.,.14);float sr=.2+uVoice*.04;float d=length(sc);
    float stripes=sc.y<0.?step(.5,fract(sc.y*28.+uTime*.3)):1.;
    vec3 sun=mix(rainbow(hueAt(.1)),voiceColor(.1,uPitch),clamp(uVoice*1.4,0.,1.));
    col+=sun*smoothstep(sr,sr-.006,d)*stripes*(.75+uVoice*.4);
    col+=sun*exp(-d*d*18.)*(.12+uVoice*.25);
    float m=abs(uv.x)/asp*2.;float mh=.03+hist(m)*.16*(.5+.5*noise(vec2(uv.x*6.,1.)));
    if(uv.y<mh){col=mix(vec3(.05,.02,.1),vec3(.12,.05,.2),uv.y/mh);col+=lowColor(m*.3)*(smoothstep(.004,0.,abs(uv.y-mh))+exp(-(mh-uv.y)*40.)*.25)*(.25+uLow*.8);}
  }else{
    float d=-uv.y;float z=.32/d;float x=uv.x*z;
    vec2 g=vec2(x,z+uTravel);vec2 w=fwidth(g);
    float lines=max(smoothstep(1.-w.x*3.,1.,1.-abs(fract(g.x+.5)-.5)*2.),smoothstep(1.-w.y*3.,1.,1.-abs(fract(g.y+.5)-.5)*2.));
    float fade=exp(-z*.06);
    col=vec3(.03,.0,.07)+rainbow(hueAt(.3+z*.012+x*.01))*lines*fade*(.35+uInt*.4);
    for(int i=0;i<8;i++){float zw=30.-uKickAge[i]*26.;float k=(z-zw)*.35;col+=lowColor(float(i)*.13)*(exp(-k*k*6.)*(.4+lines*1.2)+exp(-k*k*.8)*.25)*uKickAmp[i]*fade*step(0.,zw);}
    col+=lowColor(.6)*exp(-d*d*400.)*uLow*.35;
  }
  gl_FragColor=vec4(col,1.);
}`;

export const makeHorizon: MakeScene = (renderer) => {
  const kicks = new Ripples(8);
  const hist = new Array(HIST).fill(0);
  const U = {
    uTravel: { value: 0 }, uLow: { value: 0 }, uVoice: { value: 0 }, uPitch: { value: 0.5 }, uHat: { value: 0 }, uInt: { value: 0.3 },
    uKickAge: { value: kicks.ages }, uKickAmp: { value: kicks.amps }, uHist: { value: hist },
  };
  const s = shaderScene(THREE, renderer, FRAG, U);
  const low = new Ease(3, 1), voice = new Ease(3, 1.2), hat = new Ease(8, 2), inten = new Ease(0.6, 0.6), pitch = new Ease(2, 2), lvl = new Ease(6, 2);
  let t = 0, travel = 0, histClock = 0;
  return {
    frame({ dt, f, hit, calm, width, height }: Frame) {
      const k = calm ? 0.3 : 1;
      t += dt * k;
      if (hit.kick) kicks.add((0.6 + f.bass * 0.4) * (calm ? 0.4 : 1));
      kicks.step(dt);
      U.uInt.value = inten.to(f.intensity, dt);
      travel += dt * k * (0.6 + (U.uInt.value as number) * 1.6);
      U.uTravel.value = travel;
      U.uLow.value = low.to(f.bass, dt);
      U.uVoice.value = voice.to(f.vocal, dt);
      U.uPitch.value = pitch.to(f.pitch, dt);
      U.uHat.value = hat.to(f.hat, dt);
      // the mountains: the music's level over the last ~6 s, newest in the middle, older toward the edges
      const level = lvl.to(f.level * 0.6 + f.bass * 0.4, dt);
      histClock += dt;
      while (histClock > 0.1) {
        histClock -= 0.1;
        hist.pop();
        hist.unshift(level);
      }
      s.render(width, height, t);
    },
    dispose: () => s.dispose(),
  };
};
