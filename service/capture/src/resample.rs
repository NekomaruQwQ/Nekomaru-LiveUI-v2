//! GPU-accelerated texture resampler using a fullscreen quad.
//!
//! Draws a fullscreen quad (two triangles, 6 vertices) that samples the source
//! texture with linear filtering.  The caller controls the output region by
//! setting the viewport before calling [`Resampler::resample`].

use nkcore::prelude::*;
use nkcore::debug::*;
use nkcore::*;

use windows::Win32::Graphics::{
    Direct3D::*,
    Direct3D11::*,
};

/// Compiled vertex + pixel shaders and a linear-clamp sampler.
pub struct Resampler {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
}

impl Resampler {
    const BYTECODE_VS: &'static [u8] =
        ngd3dcompile::include_bytecode!(
            path = "resample.hlsl",
            entry = "vs_main",
            stage = "vertex");
    const BYTECODE_PS: &'static [u8] =
        ngd3dcompile::include_bytecode!(
            path = "resample.hlsl",
            entry = "ps_main",
            stage = "pixel");
    const SAMPLER_DESC: D3D11_SAMPLER_DESC = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MipLODBias: 0.0,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D11_COMPARISON_NEVER,
        BorderColor: [0.0, 0.0, 0.0, 0.0],
        MinLOD: 0.0,
        MaxLOD: D3D11_FLOAT32_MAX,
    };

    pub fn new(device: &ID3D11Device) -> anyhow::Result<Self> {
        let vs =
            out_var_or_err(|out| api_call!(unsafe {
                device.CreateVertexShader(
                    Self::BYTECODE_VS,
                    None,
                    Some(out))
            }))?.unwrap();

        let ps =
            out_var_or_err(|out| api_call!(unsafe {
                device.CreatePixelShader(
                    Self::BYTECODE_PS,
                    None,
                    Some(out))
            }))?.unwrap();

        let sampler_desc = Self::SAMPLER_DESC;
        let sampler =
            out_var_or_err(|out| api_call!(unsafe {
                device.CreateSamplerState(
                    &raw const sampler_desc,
                    Some(out))
            }))?.unwrap();

        Ok(Self { vs, ps, sampler })
    }

    /// Draw the source texture into the target render target.
    ///
    /// The caller **must** set the viewport via `RSSetViewports` before calling
    /// this — the resampler does not set its own viewport.  This is by design:
    /// the viewport controls the aspect-ratio-preserving letterbox region.
    #[expect(clippy::multiple_unsafe_ops_per_block)]
    pub fn resample(
        &self,
        ctx: &ID3D11DeviceContext,
        source_srv: &ID3D11ShaderResourceView,
        target_rtv: &ID3D11RenderTargetView) {
        unsafe {
            ctx.OMSetRenderTargets(Some(&[Some(target_rtv.clone())]), None);
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.VSSetShader(Some(&self.vs), None);
            ctx.PSSetShader(Some(&self.ps), None);
            ctx.PSSetShaderResources(0, Some(&[Some(source_srv.clone())]));
            ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            ctx.Draw(6, 0);

            ctx.OMSetRenderTargets(Some(&[]), None);
            ctx.VSSetShader(None, None);
            ctx.PSSetShader(None, None);
            ctx.PSSetShaderResources(0, Some(&[None]));
            ctx.PSSetSamplers(0, Some(&[None]));
        }
    }
}
