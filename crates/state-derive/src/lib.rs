//! Derives ZiGBA-compatible value and schema traversal.
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, parse_macro_input};

#[derive(Default)]
struct Options {
    skip: bool,
    flatten: bool,
    rename: Option<String>,
    module: Option<String>,
    schema_prefix: Option<String>,
    transparent: bool,
    bits: Option<u16>,
    tag_bits: Option<u16>,
    encode_with: Option<syn::Path>,
}
fn options(attrs: &[syn::Attribute]) -> syn::Result<Options> {
    let mut result = Options::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("codec")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                result.skip = true;
            } else if meta.path.is_ident("flatten") {
                result.flatten = true;
            } else if meta.path.is_ident("rename") {
                result.rename = Some(meta.value()?.parse::<syn::LitStr>()?.value());
            } else if meta.path.is_ident("module") {
                result.module = Some(meta.value()?.parse::<syn::LitStr>()?.value());
            } else if meta.path.is_ident("schema_prefix") {
                result.schema_prefix = Some(meta.value()?.parse::<syn::LitStr>()?.value());
            } else if meta.path.is_ident("transparent") {
                result.transparent = true;
            } else if meta.path.is_ident("bits") {
                result.bits = Some(meta.value()?.parse::<syn::LitInt>()?.base10_parse()?);
            } else if meta.path.is_ident("tag_bits") {
                result.tag_bits = Some(meta.value()?.parse::<syn::LitInt>()?.base10_parse()?);
            } else if meta.path.is_ident("encode_with") {
                result.encode_with = Some(meta.value()?.parse()?);
            } else {
                return Err(meta.error("unknown codec option"));
            }
            Ok(())
        })?;
    }
    Ok(result)
}
fn name(id: &syn::Ident, o: &Options) -> String {
    o.rename.clone().unwrap_or_else(|| id.to_string().trim_start_matches("r#").to_owned())
}
fn snake(id: &syn::Ident, o: &Options) -> String {
    if let Some(n) = &o.rename {
        return n.clone();
    }
    let chars: Vec<_> = id.to_string().chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase()
            && i > 0
            && (chars[i - 1].is_lowercase()
                || (chars[i - 1].is_uppercase()
                    && chars.get(i + 1).is_some_and(|x| x.is_lowercase())))
        {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

#[proc_macro_derive(Codec, attributes(codec))]
pub fn derive_codec(input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as DeriveInput);
    match expand(&item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
fn expand(item: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let ident = &item.ident;
    let (imp, ty, where_clause) = item.generics.split_for_impl();
    let container = options(&item.attrs)?;
    let methods = match &item.data {
        Data::Struct(data) => {
            let Fields::Named(fields) = &data.fields else {
                return Err(syn::Error::new_spanned(
                    &data.fields,
                    "Codec structs require named fields",
                ));
            };
            let mut enc = Vec::new();
            let mut dec = Vec::new();
            let mut schemas = Vec::new();
            let mut counts = Vec::new();
            for field in &fields.named {
                let id = field.ident.as_ref().unwrap();
                let ty = &field.ty;
                let o = options(&field.attrs)?;
                if o.skip {
                    continue;
                }
                let field_name = name(id, &o);
                counts.push(if o.flatten {
                    quote!(<#ty as ::evenfall_state::Codec>::FIELD_COUNT)
                } else {
                    quote!(1u32)
                });
                let encoding = if let Some(path) = &o.encode_with {
                    quote!(#path(self, sink))
                } else if let Some(bits) = o.bits {
                    quote!(::evenfall_state::NarrowCodec::encode_bits(&self.#id, sink, #bits))
                } else {
                    quote!(::evenfall_state::Codec::encode(&self.#id, sink))
                };
                enc.push(quote!(sink.field(#field_name, |sink| #encoding)?;));
                dec.push(if let Some(bits) = o.bits { quote!(::evenfall_state::NarrowCodec::decode_bits(&mut self.#id, source, #bits)?;) }
                    else { quote!(::evenfall_state::Codec::decode(&mut self.#id, source)?;) });
                schemas.push(if o.flatten { quote!(<#ty as ::evenfall_state::Codec>::schema_fields(sink)?;) }
                    else if let Some(bits) = o.bits { quote!(::evenfall_state::schema_name(sink, #field_name)?; <#ty as ::evenfall_state::NarrowCodec>::schema_bits(sink, #bits)?;) }
                    else { quote!(::evenfall_state::schema_field::<#ty>(sink, #field_name)?;) });
            }
            let schema_root = if container.transparent {
                let mut durable =
                    fields.named.iter().filter(|f| !options(&f.attrs).is_ok_and(|o| o.skip));
                let field = durable.next().ok_or_else(|| {
                    syn::Error::new_spanned(fields, "transparent Codec requires one durable field")
                })?;
                if durable.next().is_some() {
                    return Err(syn::Error::new_spanned(
                        fields,
                        "transparent Codec requires one durable field",
                    ));
                }
                let ty = &field.ty;
                quote!(<#ty as ::evenfall_state::Codec>::schema(sink))
            } else if let Some(module) = container.module {
                quote!(::evenfall_state::schema_name(sink, #module)?; Self::schema_fields(sink))
            } else if let Some(prefix) = container.schema_prefix {
                quote!(::evenfall_state::schema_name(sink, #prefix)?; sink.write_all(b"s")?; Self::FIELD_COUNT.encode(sink)?; Self::schema_fields(sink))
            } else {
                quote!(sink.write_all(b"s")?; Self::FIELD_COUNT.encode(sink)?; Self::schema_fields(sink))
            };
            quote! {
                fn encode(&self, sink: &mut ::evenfall_state::Sink<'_>) -> ::evenfall_state::Result<()> { #(#enc)* Ok(()) }
                fn decode(&mut self, source: &mut ::evenfall_state::Source<'_>) -> ::evenfall_state::Result<()> { #(#dec)* Ok(()) }
                fn schema(sink: &mut ::evenfall_state::Sink<'_>) -> ::evenfall_state::Result<()> { use ::evenfall_state::Codec as _; #schema_root }
                const FIELD_COUNT: u32 = 0 #(+ #counts)*;
                fn schema_fields(sink: &mut ::evenfall_state::Sink<'_>) -> ::evenfall_state::Result<()> { #(#schemas)* Ok(()) }
            }
        }
        Data::Enum(data) => enum_methods(data, &container)?,
        Data::Union(data) => {
            return Err(syn::Error::new_spanned(
                data.union_token,
                "untagged unions cannot be encoded",
            ));
        }
    };
    Ok(quote!(impl #imp ::evenfall_state::Codec for #ident #ty #where_clause { #methods }))
}
fn enum_methods(data: &syn::DataEnum, o: &Options) -> syn::Result<proc_macro2::TokenStream> {
    let plain = data.variants.iter().all(|v| matches!(v.fields, Fields::Unit));
    let bits = if plain {
        o.tag_bits.ok_or_else(|| {
            syn::Error::new_spanned(
                data.enum_token,
                "fieldless enums require #[codec(tag_bits = N)]",
            )
        })?
    } else {
        (usize::BITS - (data.variants.len().saturating_sub(1)).leading_zeros()) as u16
    };
    let mut writes = Vec::new();
    let mut reads = Vec::new();
    let mut variants = Vec::new();
    let mut payloads = Vec::new();
    let mut previous = quote!(0i128);
    for (i, v) in data.variants.iter().enumerate() {
        let tag = i as u128;
        let id = &v.ident;
        let field_name = snake(id, &options(&v.attrs)?);
        let discriminant = if plain {
            if let Some((_, expr)) = &v.discriminant {
                quote!((#expr) as i128)
            } else if i == 0 {
                quote!(0i128)
            } else {
                quote!((#previous + 1i128))
            }
        } else {
            quote!(#i as i128)
        };
        previous = discriminant.clone();
        variants.push(quote!(::evenfall_state::schema_name(sink, #field_name)?; ((#discriminant) as u128).encode_bits(sink, #bits)?;));
        if plain {
            writes.push(
                quote!(Self::#id => { ((#discriminant) as u128).encode_bits(sink, #bits)?; }),
            );
            reads.push(quote!(x if x == (#discriminant as u128) => Self::#id,));
        } else {
            match &v.fields {
                Fields::Unit => {
                    writes.push(quote!(Self::#id => { (#i as u128).encode_bits(sink, #bits)?; }));
                    reads.push(quote!(#tag => Self::#id,));
                    payloads.push(quote!(::evenfall_state::schema_name(sink, #field_name)?; <() as ::evenfall_state::Codec>::schema(sink)?;));
                }
                Fields::Unnamed(fs) if fs.unnamed.len() == 1 => {
                    let ty = &fs.unnamed.first().unwrap().ty;
                    writes.push(quote!(Self::#id(value) => { (#i as u128).encode_bits(sink, #bits)?; value.encode(sink)?; }));
                    reads.push(quote!(#tag => { let mut value: #ty = ::core::default::Default::default(); value.decode(source)?; Self::#id(value) },));
                    payloads
                        .push(quote!(::evenfall_state::schema_field::<#ty>(sink, #field_name)?;));
                }
                Fields::Named(fs) => {
                    let ids: Vec<_> = fs.named.iter().map(|f| f.ident.as_ref().unwrap()).collect();
                    let tys: Vec<_> = fs.named.iter().map(|f| &f.ty).collect();
                    let opts: Vec<_> =
                        fs.named.iter().map(|f| options(&f.attrs)).collect::<syn::Result<_>>()?;
                    let names: Vec<_> = ids.iter().zip(&opts).map(|(id, o)| name(id, o)).collect();
                    let vars: Vec<_> =
                        ids.iter().enumerate().map(|(n, _)| format_ident!("field_{n}")).collect();
                    let encs: Vec<_> = ids.iter().zip(&opts).map(|(id, o)| if let Some(bits) = o.bits {
                        quote!(::evenfall_state::NarrowCodec::encode_bits(#id, sink, #bits)?;)
                    } else { quote!(#id.encode(sink)?;) }).collect();
                    let decs: Vec<_> = vars.iter().zip(&opts).map(|(var, o)| if let Some(bits) = o.bits {
                        quote!(::evenfall_state::NarrowCodec::decode_bits(&mut #var, source, #bits)?;)
                    } else { quote!(#var.decode(source)?;) }).collect();
                    let fieldschemas: Vec<_> = tys.iter().zip(&names).zip(&opts).map(|((ty, name), o)| if let Some(bits) = o.bits {
                        quote!(::evenfall_state::schema_name(sink, #name)?; <#ty as ::evenfall_state::NarrowCodec>::schema_bits(sink, #bits)?;)
                    } else { quote!(::evenfall_state::schema_field::<#ty>(sink, #name)?;) }).collect();
                    let count = ids.len() as u32;
                    writes.push(quote!(Self::#id { #(#ids),* } => { (#i as u128).encode_bits(sink, #bits)?; #(#encs)* }));
                    reads.push(quote!(#tag => { #(let mut #vars: #tys = ::core::default::Default::default(); #decs)* Self::#id { #(#ids: #vars),* } },));
                    payloads.push(quote!(::evenfall_state::schema_name(sink, #field_name)?; sink.write_all(b"s")?; #count.encode(sink)?; #(#fieldschemas)*));
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        &v.fields,
                        "union variants require zero or one tuple field, or named fields",
                    ));
                }
            }
        }
    }
    let count = data.variants.len() as u32;
    let schema = if plain {
        quote!(sink.write_all(b"e")?; sink.write_all(b"u")?; #bits.encode(sink)?; #count.encode(sink)?; #(#variants)* Ok(()))
    } else {
        quote!(sink.write_all(b"v")?; sink.write_all(b"e")?; sink.write_all(b"u")?; #bits.encode(sink)?; #count.encode(sink)?; #(#variants)* #count.encode(sink)?; #(#payloads)* Ok(()))
    };
    Ok(quote! {
        fn encode(&self, sink: &mut ::evenfall_state::Sink<'_>) -> ::evenfall_state::Result<()> { use ::evenfall_state::{Codec as _, NarrowCodec as _}; match self { #(#writes)* } Ok(()) }
        fn decode(&mut self, source: &mut ::evenfall_state::Source<'_>) -> ::evenfall_state::Result<()> { use ::evenfall_state::{Codec as _, NarrowCodec as _}; let mut tag = 0u128; tag.decode_bits(source, #bits)?; *self = match tag { #(#reads)* _ => return Err(::evenfall_state::Error::InvalidSaveState) }; Ok(()) }
        fn schema(sink: &mut ::evenfall_state::Sink<'_>) -> ::evenfall_state::Result<()> { use ::evenfall_state::Codec as _; use ::evenfall_state::NarrowCodec as _; #schema }
    })
}
