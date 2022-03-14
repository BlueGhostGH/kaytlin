use chumsky::{
    primitive::{choice, end, just},
    recursive::recursive,
    select, Parser,
};

use crate::{
    ast,
    node::SrcNode,
    token::{Delimiter, Token},
};

mod parse {
    use chumsky::error::Simple;

    use crate::{node::SrcNode, span::Span, token::Token};

    pub trait Parser<T> = chumsky::Parser<Token, SrcNode<T>, Error = Simple<Token, Span>> + Clone;
}

pub fn ident_parser() -> impl parse::Parser<ast::Ident> {
    select! {
        Token::Ident(id) => id,
    }
    .map_with_span(SrcNode::new)
}

pub fn lit_parser() -> impl parse::Parser<ast::Lit> {
    select! {
        Token::Int(int) => ast::Lit::Int(int),
        Token::Str(r#str) => ast::Lit::Str(r#str),
    }
    .map_with_span(SrcNode::new)
}

pub fn path_parser() -> impl parse::Parser<ast::Path> {
    ident_parser()
        .separated_by(just([Token::Colon, Token::Colon]))
        .at_least(1)
        .allow_leading()
        .map(|segments| ast::Path { segments })
        .map_with_span(SrcNode::new)
}

pub fn ty_parser() -> impl parse::Parser<ast::Ty> {
    recursive(|ty| {
        let path = path_parser().map(ast::Ty::Path);

        let ptr = just(Token::Star)
            .ignore_then(ty)
            .map_with_span(|ty, span| ast::Ty::Ptr(SrcNode::new(ty, span)));

        path.or(ptr)
    })
    .map_with_span(SrcNode::new)
}

pub fn expr_parser() -> impl parse::Parser<ast::Expr> {
    recursive(|expr| {
        let paren_expr_list = expr
            .clone()
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .delimited_by(
                just(Token::Open(Delimiter::Paren)),
                just(Token::Close(Delimiter::Paren)),
            )
            .boxed();

        let lit = lit_parser().map(ast::Expr::Lit);
        let path = path_parser().map(ast::Expr::Path);

        let atom = lit.or(path).map_with_span(SrcNode::new).boxed();

        let call = atom
            .then(paren_expr_list.or_not())
            .map(|(expr, args)| match args {
                Some(args) => {
                    let span = args
                        .iter()
                        .fold(expr.span(), |span, arg: &SrcNode<ast::Expr>| {
                            span.union(arg.span())
                        });

                    SrcNode::new(ast::Expr::Call(expr, args), span)
                }
                None => expr,
            })
            .boxed();

        let op = just(Token::And).map_with_span(SrcNode::new);
        let addr = op.repeated().then(call).foldr(|op, expr| {
            let span = op.span().union(expr.span());
            SrcNode::new(ast::Expr::Addr(expr), span)
        });

        let op = just(Token::Star)
            .to(ast::BinOp::Mul)
            .or(just(Token::Slash).to(ast::BinOp::Div))
            .or(just(Token::Percent).to(ast::BinOp::Rem))
            .map_with_span(SrcNode::new);
        let product = addr
            .clone()
            .then(op.then(addr).repeated())
            .foldl(|a, (op, b)| {
                let span = a.span().union(b.span());
                SrcNode::new(ast::Expr::Binary(op, a, b), span)
            })
            .boxed();

        let op = just(Token::Plus)
            .to(ast::BinOp::Add)
            .or(just(Token::Minus).to(ast::BinOp::Sub))
            .map_with_span(SrcNode::new);
        let sum = product
            .clone()
            .then(op.then(product).repeated())
            .foldl(|a, (op, b)| {
                let span = a.span().union(b.span());
                SrcNode::new(ast::Expr::Binary(op, a, b), span)
            })
            .boxed();

        sum
    })
}

pub fn parser() -> impl parse::Parser<Vec<SrcNode<ast::Item>>> {
    let item = recursive(|item| {
        let init = ident_parser()
            .then_ignore(just(Token::Colon))
            .then(ty_parser().or_not())
            .then_ignore(just(Token::Eq))
            .then(expr_parser())
            .map_with_span(|((ident, ty), expr), span| {
                let kind = ast::LocalKind::Init(expr, ty);

                ast::Local {
                    ident,
                    kind: SrcNode::new(kind, span),
                }
            })
            .boxed();
        let decl = ident_parser()
            .then_ignore(just(Token::Colon))
            .then(ty_parser())
            .map_with_span(|(ident, ty), span| {
                let kind = ast::LocalKind::Decl(ty);

                ast::Local {
                    ident,
                    kind: SrcNode::new(kind, span),
                }
            })
            .boxed();
        let local = init.or(decl).boxed();

        let stmt = choice((
            item.map_with_span(|item, span| ast::Stmt::Item(SrcNode::new(item, span))),
            expr_parser()
                .map(ast::Stmt::Expr)
                .then_ignore(just(Token::Semicolon)),
            local
                .map_with_span(|local, span| ast::Stmt::Local(SrcNode::new(local, span)))
                .then_ignore(just(Token::Semicolon)),
        ))
        .boxed();

        let block = stmt
            .repeated()
            .delimited_by(
                just(Token::Open(Delimiter::Brace)),
                just(Token::Close(Delimiter::Brace)),
            )
            .map_with_span(|stmts, span| SrcNode::new(ast::Block { stmts }, span))
            .boxed();

        let generics = ident_parser()
            .separated_by(just(Token::Comma))
            .delimited_by(just(Token::Lt), just(Token::Gt))
            .map_with_span(|params, span| SrcNode::new(ast::Generics { params }, span))
            .or_not()
            .boxed();

        let field = ident_parser()
            .then_ignore(just(Token::Colon))
            .then(ty_parser())
            .map_with_span(|(ident, ty), span| SrcNode::new(ast::FieldDef { ident, ty }, span))
            .boxed();
        let fields = field
            .repeated()
            .delimited_by(
                just(Token::Open(Delimiter::Brace)),
                just(Token::Close(Delimiter::Brace)),
            )
            .map_with_span(SrcNode::new)
            .boxed();

        let r#struct = just(Token::Struct)
            .ignore_then(ident_parser())
            .then(generics.clone())
            .then(fields.map(Some).or(just(Token::Semicolon).to(None)))
            .map_with_span(|((ident, generics), fields), span| {
                let kind = ast::ItemKind::Struct { generics, fields };

                ast::Item {
                    ident,
                    kind: SrcNode::new(kind, span),
                }
            });

        let param = ident_parser()
            .then_ignore(just(Token::Colon))
            .then(ty_parser())
            .map_with_span(|(ident, ty), span| SrcNode::new(ast::Param { ident, ty }, span))
            .boxed();
        let params = param
            .separated_by(just(Token::Comma))
            .delimited_by(
                just(Token::Open(Delimiter::Paren)),
                just(Token::Close(Delimiter::Paren)),
            )
            .map_with_span(SrcNode::new)
            .boxed();

        let ret_ty = just(Token::RArrow)
            .ignore_then(ty_parser())
            .map(ast::FnRetTy::Ty)
            .or(just(Token::Semicolon)
                .map_with_span(|_, span| ast::FnRetTy::Default(SrcNode::new((), span))))
            .map_with_span(SrcNode::new)
            .boxed();

        let func = just(Token::Func)
            .ignore_then(ident_parser())
            .then(generics.clone())
            .then(params)
            .then(ret_ty)
            .then(block)
            .map_with_span(|((((ident, generics), inputs), output), block), span| {
                let sig = ast::FnSig { inputs, output };
                let kind = ast::ItemKind::Func {
                    generics,
                    sig,
                    block,
                };

                ast::Item {
                    ident,
                    kind: SrcNode::new(kind, span),
                }
            });

        r#struct.or(func)
    })
    .map_with_span(SrcNode::new);

    item.repeated()
        .then_ignore(end())
        .map_with_span(SrcNode::new)
}
