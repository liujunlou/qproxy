use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Data, Fields};

/// 自定义派生宏，获取对象的所有属性字段名
#[proc_macro_derive(FieldNames)]
pub fn derive_field_names(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    
    let fields = match input.data {
        Data::Struct(data) => match data.fields {
            Fields::Named(fields) => fields.named,
            _ => panic!("FieldNames 只能用于具名结构体"),
        },
        _ => panic!("FieldNames 只能用于结构体"),
    };

    let field_names: Vec<_> = fields
        .iter()
        .map(|field| {
            let field_name = field.ident.as_ref().unwrap();
            quote! { stringify!(#field_name) }
        })
        .collect();

    let expanded = quote! {
        impl #name {
            pub fn field_names() -> Vec<&'static str> {
                vec![#(#field_names),*]
            }
        }
    };

    TokenStream::from(expanded)
} 