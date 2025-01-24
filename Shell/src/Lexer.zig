// TODO: handle errors
const std = @import("std");
const Self = @This();

line: usize = 1,
column: usize = 0,
pos: usize = 0,
data: []const u8,

pub const Token = union(enum) {
    argument: []const u8,
    string_literal: []const u8,
    keyword: Keyword,
    eof,

    pub const Keyword = enum {
        @"const",
        @"var",
        @"fn",
        ret,
        @"if",
        @"else",
        @"while",
        @"for",
        @"break",
        @"continue",
        @"switch",
        @"or",
        @"and",
        not,
        end,
        @"dummy Keyword",

        const len = @intFromEnum(Keyword.@"dummy Keyword");
        /// comptime generates a keyword list, each keyword is an index into the returned value
        /// and the value is the keyword name as a string literal
        pub fn getKeywordlist() [len][]const u8 {
            var keywords: [len][]const u8 = .{""} ** len;

            inline for (keywords, 0..) |_, i| {
                const en: Keyword = @enumFromInt(i);
                keywords[i] = @tagName(en);
            }
            return keywords;
        }

        const keyword_list = getKeywordlist();
        pub fn fromString(str: []const u8) ?Keyword {
            for (keyword_list, 0..) |keyword, i| {
                if (std.mem.eql(u8, keyword, str)) {
                    return @enumFromInt(i);
                }
            }
            return null;
        }
    };

    pub fn asString(self: *const Token) []const u8 {
        return switch (self.*) {
            .eof => "<EOF>",
            .keyword => @tagName(self.keyword),
            inline else => |token| token,
        };
    }

    pub fn debug(token: *const Token) !void {
        const print = @import("libc").stdio.zprintf;
        const string = token.asString();
        const tag = @tagName(token.*);
        try print("{%.*s: %.*s} ", .{ tag.len, tag.ptr, string.len, string.ptr });
    }
};

pub fn init(data: []const u8) Self {
    return .{ .data = data };
}

fn at(self: *const Self) u8 {
    return self.data[self.pos];
}

fn eat(self: *Self) u8 {
    defer self.pos += 1;
    defer self.column += 1;
    return self.at();
}

inline fn is_eof(self: *const Self) bool {
    return self.pos >= self.data.len;
}

inline fn is_skippable(self: *const Self) bool {
    const x = self.at();
    return x == '\n' or x == ' ' or x == '\t';
}
fn string_literal(self: *Self) ?Token {
    _ = self.eat();
    const start = self.pos;
    while (!self.is_eof()) {
        if (self.at() == '"') break;
        _ = self.eat();
    }

    // unterminated string literal
    if (self.is_eof()) return null;
    const end = self.pos;
    _ = self.eat();
    return Token{ .string_literal = self.data[start..end] };
}

pub fn next(self: *Self) ?Token {
    while (!self.is_eof())
        switch (self.at()) {
            ' ', '\t' => _ = self.eat(),
            '\n' => {
                _ = self.eat();
                self.line += 1;
                self.column = 0;
            },
            else => break,
        };

    if (self.is_eof()) return null;
    switch (self.at()) {
        '"' => return self.string_literal(),
        else => {
            const start = self.pos;

            while (!self.is_eof()) {
                if (!self.is_skippable()) _ = self.eat() else break;
            }

            const lexeme = self.data[start..self.pos];

            const keyword = Token.Keyword.fromString(lexeme) orelse return Token{ .argument = lexeme };
            return .{ .keyword = keyword };
        },
    }
}
