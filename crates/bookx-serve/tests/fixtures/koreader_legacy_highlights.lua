-- KOReader before 2024: the look is in "highlight", the text and note in "bookmarks"
return {
    ["bookmarks"] = {
        [1] = {
            ["chapter"] = "One",
            ["datetime"] = "2023-05-01 10:00:00",
            ["highlighted"] = true,
            ["notes"] = "Quote about underscores",
            ["page"] = 12,
            ["pos0"] = "/body/DocFragment[2]/body/p[1]/text().0",
            ["pos1"] = "/body/DocFragment[2]/body/p[1]/text().20",
        },
        [2] = {
            ["chapter"] = "Two",
            ["datetime"] = "2023-05-02 10:00:00",
            ["highlighted"] = true,
            ["notes"] = "Quote that is plain",
            ["page"] = 30,
        },
    },
    ["doc_props"] = {
        ["authors"] = "Old Author",
        ["title"] = "Old KOReader Book",
    },
    ["highlight"] = {
        [12] = {
            [1] = {
                ["datetime"] = "2023-05-01 10:00:00",
                ["drawer"] = "underscore",
                ["pos0"] = "/body/DocFragment[2]/body/p[1]/text().0",
                ["pos1"] = "/body/DocFragment[2]/body/p[1]/text().20",
                ["text"] = "Quote about underscores",
            },
        },
    },
}
