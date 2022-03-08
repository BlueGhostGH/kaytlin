use chumsky::{
    error::Simple,
    primitive::{choice, end, filter_map, just},
    recursive::recursive,
    select, Error, Parser as ChumskyParser,
};

pub use super::{
    node::SrcNode,
    span::Span,
    token::{self, lexer, Delimiter, Token},
};

pub trait Parser<T> = chumsky::Parser<Token, T, Error = Simple<Token, Span>> + Clone;

mod ast {
    use super::super::node::SrcNode;

    #[derive(Debug)]
    pub enum Lit {
        Int(u128),
        Str(String),
    }

    #[derive(Debug, Clone)]
    pub enum BinOp {
        Add,
        Sub,
    }

    #[derive(Debug)]
    pub struct Path {
        pub segments: Vec<String>,
    }

    #[derive(Debug)]
    pub enum Expr {
        Binary(SrcNode<BinOp>, SrcNode<Self>, SrcNode<Self>),
        Lit(Lit),
        Call(SrcNode<Self>, Vec<SrcNode<Self>>),
        Path(Path),
    }

    #[derive(Debug)]
    pub enum ItemKind {
        Struct {
            generics: Option<Vec<String>>,
            fields: Option<Vec<(String, String)>>,
        },
        Func {
            inputs: Vec<(String, String)>,
            output: Option<String>,
            block: Box<Block>,
        },
    }

    #[derive(Debug)]
    pub struct Block {
        pub stmts: Vec<Stmt>,
    }

    #[derive(Debug)]
    pub struct Item {
        pub ident: String,
        pub kind: ItemKind,
    }

    #[derive(Debug)]
    pub enum Stmt {
        Item(Box<Item>),
        Expr(Box<Expr>),
    }
}

use ast::*;

pub fn lit_parser() -> impl Parser<ast::Lit> {
    select! {
        Token::Int(int) => ast::Lit::Int(int),
        Token::Str(r#str) => ast::Lit::Str(r#str),
    }
}

pub fn path_parser() -> impl Parser<ast::Path> {
    select! {
        Token::Ident(id) => id
    }
    .separated_by(just([Token::Colon, Token::Colon]))
    .at_least(1)
    .allow_leading()
    .map(|segments| Path { segments })
}

pub fn expr_parser() -> impl Parser<ast::Expr> {
    recursive(|expr| {
        let paren_expr_list = expr
            .clone()
            .map_with_span(SrcNode::new)
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .map(Some)
            .delimited_by(
                just(Token::Open(Delimiter::Paren)),
                just(Token::Close(Delimiter::Paren)),
            )
            .boxed();

        let lit = lit_parser().map(Expr::Lit);
        let path = path_parser().map(Expr::Path);

        let atom = lit.or(path).map_with_span(SrcNode::new).boxed();

        let call = atom
            .then(paren_expr_list.or_not())
            .map_with_span(|(expr, args), span| match args {
                Some(Some(args)) => {
                    let span = args.iter().fold(expr.span(), |span, arg: &SrcNode<Expr>| {
                        span.union(arg.span())
                    });

                    SrcNode::new(ast::Expr::Call(expr, args), span)
                }
                None => expr,

                _ => unreachable!(),
            })
            .boxed();

        let op = just(Token::Binary(token::BinOp::Add))
            .to(BinOp::Add)
            .or(just(Token::Binary(token::BinOp::Sub)).to(BinOp::Sub))
            .map_with_span(SrcNode::new);
        let sum = call
            .clone()
            .then(op.then(call).repeated())
            .foldl(|a, (op, b)| {
                let span = a.span().union(b.span());
                SrcNode::new(Expr::Binary(op, a, b), span)
            })
            .boxed();

        sum.map(SrcNode::into_inner)
    })
}

pub fn parser() -> impl chumsky::Parser<Token, Vec<Item>, Error = Simple<Token, Span>> {
    let expr = expr_parser();

    let item = recursive(|item| {
        let ident = filter_map(|span, tok| {
            if let Token::Ident(id) = tok {
                Ok(id)
            } else {
                Err(Simple::expected_input_found(span, Vec::new(), Some(tok)))
            }
        });

        let field = ident.then_ignore(just(Token::Colon)).then(ident);
        let fields = field
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .delimited_by(
                just(Token::Open(Delimiter::Brace)),
                just(Token::Close(Delimiter::Brace)),
            );

        let generics = ident
            .separated_by(just(Token::Comma))
            .delimited_by(just(Token::Lt), just(Token::Gt))
            .or_not();

        let r#struct = just(Token::Struct)
            .ignore_then(ident)
            .then(generics)
            .then(fields.map(Some).or(just(Token::Semicolon).to(None)))
            .map(|((name, generics), fields)| Item {
                ident: name,
                kind: ItemKind::Struct { generics, fields },
            });

        let arg = ident.then_ignore(just(Token::Colon)).then(ident);
        let args = arg.separated_by(just(Token::Comma)).delimited_by(
            just(Token::Open(Delimiter::Paren)),
            just(Token::Close(Delimiter::Paren)),
        );

        let ret_ty = just(Token::RArrow).ignore_then(ident);

        let block = just(Token::Open(Delimiter::Brace))
            .ignore_then(
                choice((
                    item.map(|item| Stmt::Item(Box::new(item))),
                    expr.map(|expr| Stmt::Expr(Box::new(expr))),
                ))
                .repeated(),
            )
            .then_ignore(just(Token::Close(Delimiter::Brace)))
            .map(|stmts| Block { stmts });

        let func = just(Token::Func)
            .ignore_then(ident)
            .then(args)
            .then(ret_ty.or_not())
            .then(block)
            .map(|(((name, args), ret_ty), block)| Item {
                ident: name,
                kind: ItemKind::Func {
                    inputs: args,
                    output: ret_ty,
                    block: Box::new(block),
                },
            });

        r#struct.or(func)
    });

    item.repeated().then_ignore(end())
}