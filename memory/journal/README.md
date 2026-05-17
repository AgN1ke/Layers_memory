# Memory Journal

Ця тека призначена для journal operation файлів.

Journal потрібен для crash safety: якщо операція sleep, міграція або promotion обірветься посередині, ядро зможе побачити незавершену операцію і вирішити, що з нею робити.

Runtime journal-файли не комітяться в git.
