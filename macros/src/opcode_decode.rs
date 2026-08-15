use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{
    Data, DeriveInput, Fields, Ident, LitInt, Token, Type,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
};

pub fn derive_decode(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

// ---------------------------------------------------------------------
// Attribute parsing
// ---------------------------------------------------------------------

enum VariantDecodeAttr {
    Short { base: u8, count: u8 },
    Wide { op: u8 },
    Invalid,
    Skip,
}

impl Parse for VariantDecodeAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        match ident.to_string().as_str() {
            "invalid" => Ok(VariantDecodeAttr::Invalid),
            "skip" => Ok(VariantDecodeAttr::Skip),
            "short" => {
                let content;
                syn::parenthesized!(content in input);
                let pairs = parse_kv_pairs(&content)?;
                let base = take_u8(&pairs, "base", &ident)?;
                let count = take_u8(&pairs, "count", &ident)?;
                Ok(VariantDecodeAttr::Short { base, count })
            }
            "wide" => {
                let content;
                syn::parenthesized!(content in input);
                let pairs = parse_kv_pairs(&content)?;
                let op = take_u8(&pairs, "op", &ident)?;
                Ok(VariantDecodeAttr::Wide { op })
            }
            other => Err(syn::Error::new(
                ident.span(),
                format!(
                    "unknown #[decode(..)] directive `{other}` (expected short/wide/invalid/skip)"
                ),
            )),
        }
    }
}

struct EnumDecodeAttr {
    wide_prefix: Option<u8>,
    fallback: Option<Ident>,
}

impl Parse for EnumDecodeAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let pairs = parse_kv_pairs(input)?;
        let wide_prefix = pairs
            .iter()
            .find(|(k, _)| k == "wide_prefix")
            .map(|(_, v)| lit_int_to_u8(v))
            .transpose()?;
        let fallback = pairs
            .iter()
            .find(|(k, _)| k == "fallback")
            .map(|(_, v)| expr_to_ident(v))
            .transpose()?;
        Ok(EnumDecodeAttr {
            wide_prefix,
            fallback,
        })
    }
}

/// Parses a comma-separated `key = value` list, e.g. `base = 0x1a, count = 4`.
fn parse_kv_pairs(input: ParseStream) -> syn::Result<Vec<(String, syn::Expr)>> {
    let pairs: Punctuated<syn::MetaNameValue, Token![,]> =
        Punctuated::parse_terminated_with(input, syn::MetaNameValue::parse)?;
    Ok(pairs
        .into_iter()
        .map(|p| {
            let key = p
                .path
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            (key, p.value)
        })
        .collect())
}

fn take_u8(pairs: &[(String, syn::Expr)], key: &str, ctx: &Ident) -> syn::Result<u8> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| lit_int_to_u8(v))
        .transpose()?
        .ok_or_else(|| {
            syn::Error::new(
                ctx.span(),
                format!("missing `{key}` in #[decode({ctx}(..))]"),
            )
        })
}

fn lit_int_to_u8(expr: &syn::Expr) -> syn::Result<u8> {
    if let syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Int(i),
        ..
    }) = expr
    {
        i.base10_parse::<u8>()
    } else {
        Err(syn::Error::new_spanned(expr, "expected an integer literal"))
    }
}

fn expr_to_ident(expr: &syn::Expr) -> syn::Result<Ident> {
    if let syn::Expr::Path(p) = expr {
        p.path
            .get_ident()
            .cloned()
            .ok_or_else(|| syn::Error::new_spanned(p, "expected a plain identifier"))
    } else {
        Err(syn::Error::new_spanned(expr, "expected a plain identifier"))
    }
}

fn hex_u8_lit(v: u8) -> LitInt {
    LitInt::new(&format!("{v:#04x}u8"), Span::call_site())
}

// ---------------------------------------------------------------------
// Field -> operand-read expression
// ---------------------------------------------------------------------

/// How a field's raw bytes are pulled off the reader. All of the JVM's
/// operand encodings we deal with are either read directly, or read as their
/// unsigned counterpart and then cast (that's how `i8`/`i16` operands are
/// actually encoded in a `.class` file).
fn field_decode_expr(ty: &Type) -> syn::Result<TokenStream2> {
    let name = quote!(#ty).to_string().replace(' ', "");
    match name.as_str() {
        "u8" => Ok(quote!(opnd!(u8))),
        "i8" => Ok(quote!(opnd!(u8) as i8)),
        "u16" => Ok(quote!(opnd!(u16))),
        "i16" => Ok(quote!(opnd!(u16) as i16)),
        other => Err(syn::Error::new_spanned(
            ty,
            format!(
                "#[derive(Decode)] doesn't know how to read a `{other}` field yet — \
                 add a case to `field_decode_expr` in opcode_decode_derive, or mark \
                 this variant #[decode(skip)] and decode it by hand"
            ),
        )),
    }
}

// ---------------------------------------------------------------------
// Codegen
// ---------------------------------------------------------------------

fn expand(input: DeriveInput) -> syn::Result<TokenStream2> {
    let enum_ident = &input.ident;
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input,
            "#[derive(Decode)] only supports enums",
        ));
    };

    let mut enum_attr = EnumDecodeAttr {
        wide_prefix: None,
        fallback: None,
    };
    for attr in &input.attrs {
        if attr.path().is_ident("decode") {
            enum_attr = attr.parse_args::<EnumDecodeAttr>()?;
        }
    }

    let mut invalid_ident: Option<Ident> = None;
    let mut outer_arms: Vec<TokenStream2> = Vec::new();
    let mut wide_arms: Vec<TokenStream2> = Vec::new();

    for variant in &data.variants {
        let var_attrs = variant
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("decode"))
            .map(|a| a.parse_args::<VariantDecodeAttr>())
            .collect::<syn::Result<Vec<_>>>()?;

        if var_attrs
            .iter()
            .any(|a| matches!(a, VariantDecodeAttr::Skip))
        {
            continue;
        }
        if var_attrs
            .iter()
            .any(|a| matches!(a, VariantDecodeAttr::Invalid))
        {
            invalid_ident = Some(variant.ident.clone());
            continue;
        }

        let ident = &variant.ident;
        let field_types: Vec<&Type> = match &variant.fields {
            Fields::Unnamed(f) => f.unnamed.iter().map(|f| &f.ty).collect(),
            Fields::Unit => Vec::new(),
            Fields::Named(_) => {
                return Err(syn::Error::new_spanned(
                    &variant.fields,
                    "#[derive(Decode)] doesn't support named fields",
                ));
            }
        };
        let field_decodes = field_types
            .iter()
            .map(|t| field_decode_expr(t))
            .collect::<syn::Result<Vec<_>>>()?;
        let ctor = if field_decodes.is_empty() {
            quote!(Self::#ident)
        } else {
            quote!(Self::#ident(#(#field_decodes),*))
        };

        // Wide variants have no discriminant of their own; they're only
        // reachable through the `wide_prefix` opcode's inner match.
        if let Some(VariantDecodeAttr::Wide { op }) = var_attrs
            .iter()
            .find(|a| matches!(a, VariantDecodeAttr::Wide { .. }))
        {
            let op_lit = hex_u8_lit(*op);
            wide_arms.push(quote! { #op_lit => Some(#ctor), });
            continue;
        }

        // Everything else needs its own discriminant.
        let discr = variant
            .discriminant
            .as_ref()
            .map(|(_, e)| e)
            .ok_or_else(|| {
                syn::Error::new_spanned(
                    ident,
                    "variant needs an explicit discriminant (`= 0x..`), or a \
                 #[decode(wide(..))] / #[decode(invalid)] / #[decode(skip)] attribute",
                )
            })?;
        outer_arms.push(quote! { #discr => Some(#ctor), });

        if let Some(VariantDecodeAttr::Short { base, count }) = var_attrs
            .iter()
            .find(|a| matches!(a, VariantDecodeAttr::Short { .. }))
        {
            if field_types.len() != 1 {
                return Err(syn::Error::new_spanned(
                    ident,
                    "#[decode(short(..))] only supports variants with exactly one field",
                ));
            }
            let field_ty = field_types[0];
            let base_lit = hex_u8_lit(*base);
            let end_lit = hex_u8_lit(base + count - 1);
            outer_arms.push(quote! {
                #base_lit..=#end_lit => Some(Self::#ident((op - #base_lit) as #field_ty)),
            });
        }
    }

    let invalid_ident = invalid_ident.ok_or_else(|| {
        syn::Error::new_spanned(
            enum_ident,
            "exactly one variant must be marked #[decode(invalid)] \
             (e.g. `Invalid(u8)`) to serve as the fallback",
        )
    })?;

    if let Some(prefix) = enum_attr.wide_prefix {
        let prefix_lit = hex_u8_lit(prefix);
        outer_arms.push(quote! {
            #prefix_lit => {
                let widened_op = opnd!(u8);
                match widened_op {
                    #(#wide_arms)*
                    _ => Some(Self::#invalid_ident(op)),
                }
            }
        });
    }

    let fallback_arm = if let Some(fallback) = &enum_attr.fallback {
        quote! { _ => Self::#fallback(op, reader, pc), }
    } else {
        quote! { _ => Some(Self::#invalid_ident(op)), }
    };

    Ok(quote! {
        impl #enum_ident {
            /// Decodes a single instruction, given its opcode byte (already
            /// consumed from `reader`) and a reader positioned right after it.
            /// `pc` is advanced by the number of operand bytes consumed.
            pub fn decode(op: u8, reader: &mut ClassByteReader, pc: &mut u16) -> Option<Self> {
                macro_rules! opnd {
                    ($t:ty) => {{
                        let ::core::result::Result::Ok(opr): ::core::result::Result<$t, _> = reader.decode() else {
                            return Some(Self::#invalid_ident(op));
                        };
                        *pc += ::core::mem::size_of::<$t>() as u16;
                        opr
                    }};
                }
                match op {
                    #(#outer_arms)*
                    #fallback_arm
                }
            }
        }
    })
}
